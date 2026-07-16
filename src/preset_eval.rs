//! Deterministic evaluation of versioned policy/binding task fixtures.
//!
//! Fixtures describe work, never outcomes. The evaluator generates every
//! metric, domain check, route proof, result artifact, and result hash from
//! production composition, transition, and route-audit functions.

use crate::preset::{BoundProfile, HostBindingV1, compose_preset, parse_host_binding, sha256};
use crate::preset_catalog::{builtin_binding, builtin_policy};
use crate::route_audit::{
    ContextForkMode, EnforcementState, EvidenceSource, ForkDimension, MeteredDimension,
    RouteMetering, RouteObservation, RouteStage, RouteTransition, RouteTransitionKind,
    StringDimension, VersionReference, effective_route_matches_requested,
    validate_route_observation,
};
use crate::usage_policy::{
    AvailabilityTrigger, MeteringMode, QualityTrigger, QuotaTrigger, RetryTrigger, RiskLevel,
    SafetyTrigger, TransitionDecision, TransitionRejectionReason, TransitionRequest,
    TransitionState, UsageObservation, UsagePolicyV1, parse_policy, resolve_transition,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

const SUITE_SOURCE: &str = include_str!("../evaluations/preset-suite-v1.toml");
const SOL_TERRA_LUNA_SOURCE: &str = include_str!("../evaluations/sol-terra-luna-codex-v2.toml");
const MAX_LIVE_ARTIFACT_BYTES: u64 = 1_048_576;
static LIVE_WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default)]
pub(crate) struct EvaluationOptions {
    pub(crate) at_unix: Option<u64>,
    pub(crate) host: Option<String>,
    pub(crate) live_host: Option<LiveHostCommand>,
    pub(crate) trusted_telemetry: Option<TrustedTelemetryCommand>,
}

#[derive(Clone, Debug)]
pub(crate) struct LiveHostCommand {
    pub(crate) executable: std::path::PathBuf,
    pub(crate) args: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct TrustedTelemetryCommand {
    pub(crate) registry: PathBuf,
    pub(crate) signer_id: String,
    pub(crate) collector: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationSuite {
    schema_version: u32,
    id: String,
    version: String,
    verified_at_unix: u64,
    expires_at_unix: u64,
    thresholds: EvaluationThresholds,
    candidates: Vec<CandidateFixture>,
    tasks: Vec<TaskFixture>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationThresholds {
    minimum_runs: u32,
    minimum_quality_score_bps: u32,
    minimum_reliability_bps: u32,
    maximum_average_credits_micros: u64,
    maximum_p95_latency_ms: u64,
    maximum_transition_contract_failures: u32,
    maximum_safety_stop_failures: u32,
    require_verified_routes: bool,
    require_result_hashes: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateFixture {
    id: String,
    policy: String,
    binding: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TaskFixture {
    id: String,
    version: String,
    kind: TaskKind,
    objective: String,
    input: String,
    artifact_kind: String,
    expected_output: String,
    work_units: u64,
    transition: FixtureTransition,
    requires_write: bool,
    minimum_tool_budget: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TaskKind {
    Exploration,
    Implementation,
    Mechanical,
    Browser,
    Visual,
    Security,
    Subagent,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum FixtureTransition {
    Retry,
    AvailabilityFallback,
    QualityEscalation,
    QuotaDowngrade,
    SafetyStop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvaluationStatus {
    Unverified,
    Verified,
    Recommended,
    Stale,
    Incompatible,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PresetEvaluationReport {
    pub(crate) schema_version: u32,
    pub(crate) suite: SuiteProvenance,
    pub(crate) environment: EnvironmentProvenance,
    pub(crate) thresholds: EvaluationThresholds,
    pub(crate) task_fixtures: Vec<TaskProvenance>,
    pub(crate) candidates: Vec<CandidateReport>,
    pub(crate) transition_counts: BTreeMap<String, TransitionCount>,
    pub(crate) codex_dispatch_contract: CodexDispatchContract,
    pub(crate) recommended: Vec<Recommendation>,
    pub(crate) reproducible_evidence: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SuiteProvenance {
    id: String,
    version: String,
    verified_at_unix: u64,
    expires_at_unix: u64,
    evaluated_at_unix: u64,
    fixture_sha256: String,
    sol_terra_luna_fixture_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EnvironmentProvenance {
    runner: String,
    runner_schema_version: u32,
    planr_version: String,
    operating_system: String,
    architecture: String,
    requested_host: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TaskProvenance {
    id: String,
    version: String,
    kind: TaskKind,
    input_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CandidateReport {
    id: String,
    policy: VersionReference,
    binding: VersionReference,
    model_versions: BTreeMap<String, ModelProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_host: Option<VersionReference>,
    status: EvaluationStatus,
    status_reasons: Vec<String>,
    metrics: CandidateMetrics,
    threshold_results: Vec<ThresholdResult>,
    evidence_scope: EvidenceScope,
    evidence_complete: bool,
    recommendation_evidence_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    results: Vec<ResultArtifact>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ModelProvenance {
    client: String,
    model: String,
    effort: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CandidateMetrics {
    metrics_source: MetricsSource,
    runs: u32,
    actual_task_runs: u32,
    oracle_passes: u32,
    average_quality_score_bps: u32,
    reliability_bps: u32,
    total_tool_calls: u64,
    total_tokens: u64,
    total_credits_micros: u64,
    average_credits_micros: u64,
    p95_latency_ms: u64,
    retries: u32,
    availability_fallbacks: u32,
    quality_escalations: u32,
    corrections: u32,
    violations: u32,
    transition_contract_failures: u32,
    safety_stop_attempts: u32,
    safety_stop_failures: u32,
    verified_route_runs: u32,
    verified_result_hashes: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThresholdResult {
    name: String,
    rule: String,
    actual: String,
    pass: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultArtifact {
    task: VersionReference,
    task_kind: TaskKind,
    input_sha256: String,
    result_sha256: String,
    evidence: EvaluationEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceScope {
    PolicySimulation,
    LiveTaskRun,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MetricsSource {
    EstimatedProjection,
    HostReported,
    TrustedTelemetry,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OracleEvidence {
    kind: String,
    expected_sha256: String,
    actual_sha256: String,
    pass: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EvaluationEvidence {
    schema_version: u32,
    generated_by: String,
    evidence_scope: EvidenceScope,
    task_executed: bool,
    outcome_oracle_evaluated: bool,
    recommendation_eligible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<VersionReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oracle: Option<OracleEvidence>,
    candidate_id: String,
    task_id: String,
    task_kind: TaskKind,
    checks: Vec<GeneratedCheck>,
    quality_score_bps: u32,
    successful: bool,
    metrics_source: MetricsSource,
    metering_confidence: MeteringMode,
    tool_calls: u64,
    tokens: u64,
    credits_micros: u64,
    latency_ms: u64,
    retries: u32,
    availability_fallbacks: u32,
    quality_escalations: u32,
    corrections: u32,
    violations: u32,
    transition: String,
    transition_allowed: bool,
    transition_contract_pass: bool,
    route_verified: bool,
    route_observation: RouteObservation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedCheck {
    name: String,
    pass: bool,
    detail: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct TransitionCount {
    attempted: u32,
    allowed: u32,
    rejected: u32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CodexDispatchContract {
    fixture: VersionReference,
    model_versions: BTreeMap<String, ModelProvenance>,
    all_fork_rejected: bool,
    none_fork_parameters_verified: bool,
    missing_effective_evidence_cannot_verify: bool,
    verified_effective_evidence_passes: bool,
    errors: Vec<String>,
    pass: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Recommendation {
    policy: String,
    binding: String,
    status: EvaluationStatus,
    label: String,
    fixture_id: String,
    fixture_version: String,
    fixture_sha256: String,
    result_hashes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RegistryEvaluationValidation {
    pub(crate) recommended: bool,
}

#[derive(Clone, Debug, Serialize)]
struct LiveHostRequest {
    schema_version: u32,
    suite: VersionReference,
    candidate: VersionReference,
    policy: VersionReference,
    binding: VersionReference,
    task: VersionReference,
    task_kind: TaskKind,
    requested_route: RequestedRoute,
    input: String,
    input_sha256: String,
    artifact_kind: String,
    workspace_path: String,
    challenge_path: String,
    artifact_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveHostResponse {
    schema_version: u32,
    host_id: String,
    host_version: String,
    candidate_id: String,
    task_id: String,
    input_sha256: String,
    artifact_kind: String,
    #[serde(default)]
    artifact_sha256: String,
    output: String,
    effective_model: String,
    effective_effort: Option<String>,
    effective_context_fork: ContextForkMode,
    effective_agent_type: String,
    #[serde(default)]
    effective_role: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    tool_calls: u64,
    tokens: u64,
    credits_micros: u64,
    retries: u32,
    availability_fallbacks: u32,
    quality_escalations: u32,
    corrections: u32,
    violations: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LiveTaskChallenge {
    schema_version: u32,
    nonce: String,
    candidate_id: String,
    task_id: String,
    input_sha256: String,
    artifact_kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LiveTaskArtifact {
    schema_version: u32,
    candidate_id: String,
    task_id: String,
    input_sha256: String,
    artifact_kind: String,
    challenge_sha256: String,
    output: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustedTelemetryPayload {
    schema_version: u32,
    run_id: String,
    evaluated_at_unix: u64,
    suite_id: String,
    suite_version: String,
    candidate_id: String,
    task_id: String,
    input_sha256: String,
    artifact_kind: String,
    artifact_sha256: String,
    challenge_nonce: String,
    requested_route: RequestedRoute,
    host_id: String,
    host_version: String,
    effective_model: String,
    effective_effort: Option<String>,
    effective_context_fork: ContextForkMode,
    effective_agent_type: String,
    #[serde(default)]
    effective_role: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    tool_calls: u64,
    tokens: u64,
    credits_micros: u64,
    retries: u32,
    availability_fallbacks: u32,
    quality_escalations: u32,
    corrections: u32,
    violations: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedTelemetryReceipt {
    payload: TrustedTelemetryPayload,
    signature_hex: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedTelemetryRegistryFile {
    schema_version: u32,
    signers: Vec<TrustedTelemetrySignerConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedTelemetrySignerConfig {
    id: String,
    public_key_hex: String,
    collector_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct TelemetryCollectorRequest {
    schema_version: u32,
    run_id: String,
    evaluated_at_unix: u64,
    suite_id: String,
    suite_version: String,
    candidate_id: String,
    task_id: String,
    input_sha256: String,
    artifact_kind: String,
    artifact_sha256: String,
    challenge_nonce: String,
    requested_route: RequestedRoute,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RequestedRoute {
    role: String,
    agent_type: Option<String>,
    model: String,
    effort: Option<String>,
    context_fork: ContextForkMode,
}

fn requested_route(role: &str, profile: &BoundProfile) -> RequestedRoute {
    RequestedRoute {
        role: role.to_string(),
        agent_type: profile.agent_type.clone(),
        model: profile.model.clone(),
        effort: profile.effort.clone(),
        context_fork: profile.fork_turns.clone().unwrap_or(ContextForkMode::None),
    }
}

struct VerifiedTelemetryCollector {
    public_key: VerifyingKey,
    executable: PathBuf,
    executable_sha256: String,
}

struct LiveExecutionWorkspace {
    root: PathBuf,
    challenge_path: PathBuf,
    artifact_path: PathBuf,
    challenge_sha256: String,
    challenge_nonce: String,
}

impl LiveExecutionWorkspace {
    fn create(
        candidate_id: &str,
        task_id: &str,
        input_sha256: &str,
        artifact_kind: &str,
    ) -> Result<Self, String> {
        let counter = LIVE_WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let nonce = sha256(
            format!(
                "{}:{counter}:{nanos}:{candidate_id}:{task_id}:{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            )
            .as_bytes(),
        );
        let root = std::env::temp_dir().join(format!(
            "planr-preset-eval-{}-{counter}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&root)
            .map_err(|error| format!("failed to create live task workspace: {error}"))?;
        let challenge_path = root.join("challenge.json");
        let artifact_path = root.join("artifact.json");
        let challenge = LiveTaskChallenge {
            schema_version: 1,
            nonce: nonce.clone(),
            candidate_id: candidate_id.to_string(),
            task_id: task_id.to_string(),
            input_sha256: input_sha256.to_string(),
            artifact_kind: artifact_kind.to_string(),
        };
        let challenge_bytes = serde_json::to_vec(&challenge)
            .map_err(|error| format!("failed to serialize live task challenge: {error}"))?;
        fs::write(&challenge_path, &challenge_bytes)
            .map_err(|error| format!("failed to write live task challenge: {error}"))?;
        Ok(Self {
            root,
            challenge_path,
            artifact_path,
            challenge_sha256: sha256(&challenge_bytes),
            challenge_nonce: nonce,
        })
    }

    fn read_artifact(&self) -> Option<(LiveTaskArtifact, String)> {
        let metadata = fs::symlink_metadata(&self.artifact_path).ok()?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_LIVE_ARTIFACT_BYTES {
            return None;
        }
        let bytes = fs::read(&self.artifact_path).ok()?;
        let digest = sha256(&bytes);
        let artifact = serde_json::from_slice(&bytes).ok()?;
        Some((artifact, digest))
    }
}

impl Drop for LiveExecutionWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl VerifiedTelemetryCollector {
    fn load(command: &TrustedTelemetryCommand) -> Result<Self, String> {
        if !command.collector.is_absolute() {
            return Err("trusted telemetry collector must be an absolute path".to_string());
        }
        let metadata = fs::symlink_metadata(&command.registry)
            .map_err(|error| format!("failed to read trusted telemetry registry: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("trusted telemetry registry must be a regular file".to_string());
        }
        let bytes = fs::read(&command.registry)
            .map_err(|error| format!("failed to read trusted telemetry registry: {error}"))?;
        if bytes.len() > MAX_LIVE_ARTIFACT_BYTES as usize {
            return Err("trusted telemetry registry exceeded 1 MiB".to_string());
        }
        let registry: TrustedTelemetryRegistryFile = toml::from_str(
            std::str::from_utf8(&bytes)
                .map_err(|error| format!("trusted telemetry registry was not UTF-8: {error}"))?,
        )
        .map_err(|error| format!("trusted telemetry registry was invalid: {error}"))?;
        if registry.schema_version != 1 {
            return Err(format!(
                "trusted telemetry registry schema_version {} is unsupported",
                registry.schema_version
            ));
        }
        if command.signer_id.trim().is_empty() {
            return Err("trusted telemetry signer id must not be blank".to_string());
        }
        let matching = registry
            .signers
            .iter()
            .filter(|signer| signer.id == command.signer_id)
            .collect::<Vec<_>>();
        let [signer] = matching.as_slice() else {
            return Err(format!(
                "trusted telemetry signer `{}` must resolve uniquely in {}",
                command.signer_id,
                command.registry.display()
            ));
        };
        let public_key_bytes = decode_hex::<32>(&signer.public_key_hex)
            .map_err(|error| format!("trusted telemetry registry public key: {error}"))?;
        let public_key = VerifyingKey::from_bytes(&public_key_bytes).map_err(|error| {
            format!("trusted telemetry registry public key is invalid: {error}")
        })?;
        let collector_metadata = fs::symlink_metadata(&command.collector)
            .map_err(|error| format!("failed to inspect trusted telemetry collector: {error}"))?;
        if !collector_metadata.file_type().is_file() {
            return Err("trusted telemetry collector must be a regular file".to_string());
        }
        let collector_bytes = fs::read(&command.collector)
            .map_err(|error| format!("failed to read trusted telemetry collector: {error}"))?;
        let executable_sha256 = sha256(&collector_bytes);
        if executable_sha256 != signer.collector_sha256 {
            return Err(format!(
                "trusted telemetry collector digest did not match signer `{}` registry entry",
                signer.id
            ));
        }
        Ok(Self {
            public_key,
            executable: command.collector.clone(),
            executable_sha256,
        })
    }

    fn collect(&self, request: &TelemetryCollectorRequest) -> Option<TrustedTelemetryPayload> {
        let current_bytes = fs::read(&self.executable).ok()?;
        if sha256(&current_bytes) != self.executable_sha256 {
            return None;
        }
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let request_bytes = serde_json::to_vec(request).ok()?;
        child.stdin.take()?.write_all(&request_bytes).ok()?;
        let output = child.wait_with_output().ok()?;
        if !output.status.success() || output.stdout.len() > MAX_LIVE_ARTIFACT_BYTES as usize {
            return None;
        }
        let receipt: SignedTelemetryReceipt = serde_json::from_slice(&output.stdout).ok()?;
        let signature = Signature::from_bytes(&decode_hex::<64>(&receipt.signature_hex).ok()?);
        let message = serde_json::to_vec(&receipt.payload).ok()?;
        let payload = receipt.payload;
        let identity_matches = payload.schema_version == request.schema_version
            && payload.run_id == request.run_id
            && payload.evaluated_at_unix == request.evaluated_at_unix
            && payload.suite_id == request.suite_id
            && payload.suite_version == request.suite_version
            && payload.candidate_id == request.candidate_id
            && payload.task_id == request.task_id
            && payload.input_sha256 == request.input_sha256
            && payload.artifact_kind == request.artifact_kind
            && payload.artifact_sha256 == request.artifact_sha256
            && payload.challenge_nonce == request.challenge_nonce
            && payload.requested_route == request.requested_route
            && !payload.host_id.trim().is_empty()
            && !payload.host_version.trim().is_empty();
        (identity_matches && self.public_key.verify(&message, &signature).is_ok())
            .then_some(payload)
    }
}

pub(crate) fn sign_telemetry_payload(
    private_key_file: &Path,
    input: &str,
) -> Result<serde_json::Value, String> {
    let metadata = fs::symlink_metadata(private_key_file)
        .map_err(|error| format!("failed to inspect telemetry private key: {error}"))?;
    if !metadata.file_type().is_file() || metadata.len() > 256 {
        return Err(
            "telemetry private key must be a regular file no larger than 256 bytes".to_string(),
        );
    }
    let seed_hex = fs::read_to_string(private_key_file)
        .map_err(|error| format!("failed to read telemetry private key: {error}"))?;
    let signing_key = SigningKey::from_bytes(
        &decode_hex::<32>(seed_hex.trim())
            .map_err(|error| format!("telemetry private key: {error}"))?,
    );
    let payload: TrustedTelemetryPayload = serde_json::from_str(input)
        .map_err(|error| format!("telemetry payload was invalid: {error}"))?;
    if payload.schema_version != 1 {
        return Err(format!(
            "telemetry payload schema_version {} is unsupported",
            payload.schema_version
        ));
    }
    let message = serde_json::to_vec(&payload)
        .map_err(|error| format!("failed to serialize telemetry payload: {error}"))?;
    let signature = signing_key.sign(&message);
    Ok(serde_json::json!({
        "payload": payload,
        "signature_hex": encode_hex(&signature.to_bytes()),
    }))
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], String> {
    let input = value.as_bytes();
    if input.len() != N * 2 {
        return Err(format!("expected {} hexadecimal characters", N * 2));
    }
    let mut output = [0_u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        let high =
            hex_nibble(input[offset]).ok_or_else(|| "value was not hexadecimal".to_string())?;
        let low =
            hex_nibble(input[offset + 1]).ok_or_else(|| "value was not hexadecimal".to_string())?;
        *byte = high * 16 + low;
    }
    Ok(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

struct LiveHostResult {
    response: LiveHostResponse,
    latency_ms: u64,
}

trait LiveHostAdapter {
    fn run(&self, request: &LiveHostRequest) -> Result<LiveHostResult, String>;
}

struct ProcessLiveHostAdapter<'a> {
    command: &'a LiveHostCommand,
}

impl LiveHostAdapter for ProcessLiveHostAdapter<'_> {
    fn run(&self, request: &LiveHostRequest) -> Result<LiveHostResult, String> {
        if !self.command.executable.is_absolute() {
            return Err("live host executable must be an absolute path".to_string());
        }
        let started = Instant::now();
        let mut child = Command::new(&self.command.executable)
            .args(&self.command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to start live host adapter: {error}"))?;
        let request_bytes = serde_json::to_vec(request)
            .map_err(|error| format!("failed to serialize live host request: {error}"))?;
        child
            .stdin
            .take()
            .ok_or_else(|| "live host adapter stdin was unavailable".to_string())?
            .write_all(&request_bytes)
            .map_err(|error| format!("failed to write live host request: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed to wait for live host adapter: {error}"))?;
        let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        if !output.status.success() {
            return Err(format!("live host adapter exited with {}", output.status));
        }
        if output.stdout.len() > 1_048_576 {
            return Err("live host adapter response exceeded 1 MiB".to_string());
        }
        let response: LiveHostResponse = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("live host adapter response was invalid: {error}"))?;
        if response.schema_version != 1 {
            return Err(format!(
                "live host adapter schema_version {} is unsupported",
                response.schema_version
            ));
        }
        if response.host_id.trim().is_empty() || response.host_version.trim().is_empty() {
            return Err(
                "live host adapter must report non-empty host_id and host_version".to_string(),
            );
        }
        Ok(LiveHostResult {
            response,
            latency_ms,
        })
    }
}

/// Challenge every native Codex agent type through the same independently
/// signed telemetry boundary used by live preset evaluation. A caller cannot
/// assert native-v2 or backend support: the hash-pinned collector must bind
/// each requested agent type to its observed model, effort, and none-fork.
pub(crate) fn verify_codex_binding_capabilities(
    binding: &HostBindingV1,
    live_host: &LiveHostCommand,
    telemetry: &TrustedTelemetryCommand,
    installed_version_output: &str,
) -> Result<(), String> {
    let adapter = ProcessLiveHostAdapter { command: live_host };
    let collector = VerifiedTelemetryCollector::load(telemetry)?;
    let evaluated_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let run_id = uuid::Uuid::new_v4().to_string();
    let candidate_id = format!("{}-native-capability", binding.id);
    for (role, profile) in binding
        .profiles
        .iter()
        .filter(|(_, profile)| profile.client == "codex")
    {
        let agent_type = profile
            .agent_type
            .as_deref()
            .ok_or_else(|| format!("binding role `{role}` has no native Codex agent_type"))?;
        let effort = profile
            .effort
            .as_deref()
            .ok_or_else(|| format!("binding role `{role}` has no explicit reasoning effort"))?;
        let input = serde_json::to_string(&serde_json::json!({
            "agent_type": agent_type,
            "model": profile.model,
            "effort": effort,
            "fork_turns": {"mode": "none"},
        }))
        .map_err(|error| format!("failed to serialize capability challenge: {error}"))?;
        let input_sha256 = sha256(input.as_bytes());
        let artifact_kind = "codex_native_capability";
        let workspace = LiveExecutionWorkspace::create(
            &candidate_id,
            agent_type,
            &input_sha256,
            artifact_kind,
        )?;
        let request = LiveHostRequest {
            schema_version: 1,
            suite: VersionReference {
                id: "planr-codex-capability".to_string(),
                version: "1.0.0".to_string(),
            },
            candidate: VersionReference {
                id: candidate_id.clone(),
                version: binding.version.clone(),
            },
            policy: VersionReference {
                id: "capability-probe".to_string(),
                version: "1.0.0".to_string(),
            },
            binding: VersionReference {
                id: binding.id.clone(),
                version: binding.version.clone(),
            },
            task: VersionReference {
                id: agent_type.to_string(),
                version: "1.0.0".to_string(),
            },
            task_kind: TaskKind::Subagent,
            requested_route: requested_route(role, profile),
            input,
            input_sha256: input_sha256.clone(),
            artifact_kind: artifact_kind.to_string(),
            workspace_path: workspace.root.display().to_string(),
            challenge_path: workspace.challenge_path.display().to_string(),
            artifact_path: workspace.artifact_path.display().to_string(),
        };
        let live = adapter.run(&request)?;
        let (artifact, artifact_sha256) = workspace.read_artifact().ok_or_else(|| {
            format!("native-v2 probe for `{agent_type}` produced no challenge-bound artifact")
        })?;
        if artifact.schema_version != 1
            || artifact.candidate_id != candidate_id
            || artifact.task_id != agent_type
            || artifact.input_sha256 != input_sha256
            || artifact.artifact_kind != artifact_kind
            || artifact.challenge_sha256 != workspace.challenge_sha256
            || live.response.artifact_sha256 != artifact_sha256
        {
            return Err(format!(
                "native-v2 probe for `{agent_type}` was not bound to Planr's challenge"
            ));
        }
        let receipt = collector
            .collect(&TelemetryCollectorRequest {
                schema_version: 1,
                run_id: run_id.clone(),
                evaluated_at_unix,
                suite_id: request.suite.id.clone(),
                suite_version: request.suite.version.clone(),
                candidate_id: candidate_id.clone(),
                task_id: agent_type.to_string(),
                input_sha256: input_sha256.clone(),
                artifact_kind: artifact_kind.to_string(),
                artifact_sha256,
                challenge_nonce: workspace.challenge_nonce.clone(),
                requested_route: request.requested_route.clone(),
            })
            .ok_or_else(|| {
                format!("native-v2 probe for `{agent_type}` lacks a valid signed telemetry receipt")
            })?;
        if receipt.host_id != "codex"
            || receipt.host_id != live.response.host_id
            || receipt.host_version != live.response.host_version
            || !installed_version_output
                .split_whitespace()
                .last()
                .is_some_and(|version| {
                    version.trim_start_matches('v') == receipt.host_version.trim_start_matches('v')
                })
        {
            return Err(format!(
                "native-v2 probe for `{agent_type}` must carry signed host_id `codex` and match the installed Codex version"
            ));
        }
        if receipt.effective_model != profile.model
            || receipt.effective_effort.as_deref() != Some(effort)
            || receipt.effective_context_fork != ContextForkMode::None
            || receipt.effective_agent_type != agent_type
        {
            return Err(format!(
                "active backend does not advertise `{agent_type}` as {}/{effort} with fork_turns none (observed {}/{:?})",
                profile.model, receipt.effective_model, receipt.effective_effort
            ));
        }
    }
    Ok(())
}

pub(crate) fn evaluate_embedded_suite(
    options: &EvaluationOptions,
) -> Result<PresetEvaluationReport, String> {
    evaluate_embedded_suite_with_clock(options, || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    })
}

pub(crate) fn validate_registry_evaluation(
    raw: &str,
    policy_id: &str,
    policy_raw: &str,
    binding_id: &str,
    binding_raw: &str,
    now_unix: u64,
    requested_host: Option<&str>,
) -> Result<RegistryEvaluationValidation, String> {
    let envelope: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("preset evaluation report is not valid JSON: {error}"))?;
    let report = envelope.get("report").unwrap_or(&envelope);
    if report
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err("preset evaluation report schema_version must be 1".into());
    }

    let suite: EvaluationSuite = toml::from_str(SUITE_SOURCE)
        .map_err(|error| format!("embedded evaluation suite is invalid: {error}"))?;
    validate_suite(&suite)?;
    let report_suite = report
        .get("suite")
        .ok_or_else(|| "preset evaluation report is missing suite provenance".to_string())?;
    require_json_string(report_suite, "id", &suite.id)?;
    require_json_string(report_suite, "version", &suite.version)?;
    require_json_string(
        report_suite,
        "fixture_sha256",
        &sha256(SUITE_SOURCE.as_bytes()),
    )?;
    require_json_string(
        report_suite,
        "sol_terra_luna_fixture_sha256",
        &sha256(SOL_TERRA_LUNA_SOURCE.as_bytes()),
    )?;
    require_json_u64(report_suite, "verified_at_unix", suite.verified_at_unix)?;
    require_json_u64(report_suite, "expires_at_unix", suite.expires_at_unix)?;
    let evaluated_at = json_u64(report_suite, "evaluated_at_unix")?;
    if evaluated_at < suite.verified_at_unix || evaluated_at > now_unix {
        return Err("preset evaluation report has an invalid evaluated_at_unix".into());
    }
    let suite_current = now_unix <= suite.expires_at_unix;
    let expected_thresholds = serde_json::to_value(&suite.thresholds)
        .map_err(|error| format!("failed to serialize current evaluation thresholds: {error}"))?;
    if report.get("thresholds") != Some(&expected_thresholds) {
        return Err("preset evaluation thresholds do not match the current suite".into());
    }
    validate_registry_task_provenance(report, &suite)?;

    let environment = report
        .get("environment")
        .ok_or_else(|| "preset evaluation report is missing environment provenance".to_string())?;
    require_json_string(environment, "planr_version", env!("CARGO_PKG_VERSION"))?;

    let policy_source = builtin_policy(Path::new(policy_id))
        .ok_or_else(|| format!("evaluation references unknown built-in policy `{policy_id}`"))?;
    let binding_source = builtin_binding(Path::new(binding_id))
        .ok_or_else(|| format!("evaluation references unknown built-in binding `{binding_id}`"))?;
    if sha256(policy_raw.as_bytes()) != sha256(policy_source.content.as_bytes()) {
        return Err(format!(
            "registry policy `{policy_id}` content was not the content evaluated by the current suite"
        ));
    }
    if sha256(binding_raw.as_bytes()) != sha256(binding_source.content.as_bytes()) {
        return Err(format!(
            "registry binding `{binding_id}` content was not the content evaluated by the current suite"
        ));
    }

    let candidates = report
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "preset evaluation report candidates must be an array".to_string())?;
    let candidate = candidates
        .iter()
        .find(|candidate| {
            candidate
                .pointer("/policy/id")
                .and_then(serde_json::Value::as_str)
                == Some(policy_id)
                && candidate
                    .pointer("/binding/id")
                    .and_then(serde_json::Value::as_str)
                    == Some(binding_id)
        })
        .ok_or_else(|| {
            format!("evaluation report has no candidate for `{policy_id}` + `{binding_id}`")
        })?;
    require_json_string(&candidate["policy"], "version", policy_source.version)?;
    require_json_string(&candidate["binding"], "version", binding_source.version)?;
    if candidate
        .get("evidence_complete")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return Err(
            "preset evaluation report does not contain complete reproducible evidence".into(),
        );
    }
    let expected_dispatch_contract = serde_json::to_value(evaluate_sol_terra_luna_contract()?)
        .map_err(|error| format!("failed to serialize canonical dispatch contract: {error}"))?;
    if report.get("codex_dispatch_contract") != Some(&expected_dispatch_contract) {
        return Err(
            "preset evaluation Codex dispatch contract does not match the canonical check".into(),
        );
    }
    let typed_results = validate_registry_result_hashes(candidate, &suite)?;
    let reported_metrics: CandidateMetrics =
        serde_json::from_value(candidate["metrics"].clone())
            .map_err(|error| format!("evaluation candidate metrics are invalid: {error}"))?;
    let derived_metrics = aggregate_metrics(&typed_results);
    if reported_metrics != derived_metrics {
        return Err("evaluation candidate metrics were not derived from result evidence".into());
    }
    let reported_thresholds: Vec<ThresholdResult> =
        serde_json::from_value(candidate["threshold_results"].clone())
            .map_err(|error| format!("evaluation candidate thresholds are invalid: {error}"))?;
    let derived_thresholds = threshold_results(&derived_metrics, &suite.thresholds);
    if reported_thresholds != derived_thresholds {
        return Err(
            "evaluation candidate threshold gates were not derived from current metrics".into(),
        );
    }

    let candidate_status = candidate
        .get("status")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "evaluation candidate is missing status".to_string())?;
    if !matches!(candidate_status, "verified" | "recommended") {
        return Err(format!(
            "evaluation candidate status `{candidate_status}` is not registry-eligible"
        ));
    }
    let recommended = candidate_status == "recommended" && suite_current;
    if candidate_status == "recommended" {
        validate_registry_recommendation_gates(RegistryRecommendationContext {
            report,
            candidate,
            suite: &suite,
            policy_id,
            binding_id,
            requested_host,
            results: &typed_results,
            metrics: &derived_metrics,
            thresholds: &derived_thresholds,
        })?;
    }
    Ok(RegistryEvaluationValidation { recommended })
}

fn validate_registry_task_provenance(
    report: &serde_json::Value,
    suite: &EvaluationSuite,
) -> Result<(), String> {
    let tasks = report
        .get("task_fixtures")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "preset evaluation task_fixtures must be an array".to_string())?;
    if tasks.len() != suite.tasks.len() {
        return Err("preset evaluation task fixture count does not match the current suite".into());
    }
    for expected in &suite.tasks {
        let task = tasks
            .iter()
            .find(|task| task.get("id").and_then(serde_json::Value::as_str) == Some(&expected.id))
            .ok_or_else(|| format!("evaluation report is missing task `{}`", expected.id))?;
        require_json_string(task, "version", &expected.version)?;
        require_json_string(task, "input_sha256", &hash_json(expected)?)?;
        if task.get("kind")
            != Some(
                &serde_json::to_value(expected.kind)
                    .map_err(|error| format!("failed to serialize task kind: {error}"))?,
            )
        {
            return Err(format!(
                "evaluation task `{}` kind does not match the current suite",
                expected.id
            ));
        }
    }
    Ok(())
}

fn validate_registry_result_hashes(
    candidate: &serde_json::Value,
    suite: &EvaluationSuite,
) -> Result<Vec<ResultArtifact>, String> {
    let results = candidate
        .get("results")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "evaluation candidate results must be an array".to_string())?;
    if results.len() != suite.tasks.len() {
        return Err("evaluation candidate result count does not match the current suite".into());
    }
    let mut seen = BTreeSet::new();
    let mut typed_results = Vec::new();
    for result in results {
        let task_id = result
            .pointer("/task/id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "evaluation result is missing task id".to_string())?;
        let task = suite
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| format!("evaluation result references unknown task `{task_id}`"))?;
        if !seen.insert(task_id) {
            return Err(format!("evaluation result duplicates task `{task_id}`"));
        }
        require_json_string(&result["task"], "version", &task.version)?;
        require_json_string(result, "input_sha256", &hash_json(task)?)?;
        let evidence = result
            .get("evidence")
            .ok_or_else(|| format!("evaluation result `{task_id}` is missing evidence"))?;
        let observed_hash = result
            .get("result_sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("evaluation result `{task_id}` is missing result_sha256"))?;
        let typed_evidence: EvaluationEvidence =
            serde_json::from_value(evidence.clone()).map_err(|error| {
                format!("evaluation result `{task_id}` evidence is invalid: {error}")
            })?;
        let expected_hash = hash_json(&typed_evidence)?;
        if observed_hash != expected_hash {
            return Err(format!(
                "evaluation result `{task_id}` evidence hash does not match result_sha256"
            ));
        }
        typed_results.push(
            serde_json::from_value(result.clone())
                .map_err(|error| format!("evaluation result `{task_id}` is invalid: {error}"))?,
        );
    }
    Ok(typed_results)
}

struct RegistryRecommendationContext<'a> {
    report: &'a serde_json::Value,
    candidate: &'a serde_json::Value,
    suite: &'a EvaluationSuite,
    policy_id: &'a str,
    binding_id: &'a str,
    requested_host: Option<&'a str>,
    results: &'a [ResultArtifact],
    metrics: &'a CandidateMetrics,
    thresholds: &'a [ThresholdResult],
}

fn validate_registry_recommendation_gates(
    context: RegistryRecommendationContext<'_>,
) -> Result<(), String> {
    let RegistryRecommendationContext {
        report,
        candidate,
        suite,
        policy_id,
        binding_id,
        requested_host,
        results,
        metrics,
        thresholds,
    } = context;
    for (field, expected) in [
        ("evidence_complete", true),
        ("recommendation_evidence_complete", true),
    ] {
        if candidate.get(field).and_then(serde_json::Value::as_bool) != Some(expected) {
            return Err(format!("recommended candidate requires `{field} = true`"));
        }
    }
    require_json_string(candidate, "label", "recommended")?;
    if thresholds.is_empty() || thresholds.iter().any(|gate| !gate.pass) {
        return Err("recommended candidate has a failed or missing threshold gate".into());
    }
    let runs = u64::from(metrics.runs);
    if runs != suite.tasks.len() as u64
        || u64::from(metrics.actual_task_runs) != runs
        || u64::from(metrics.oracle_passes) != runs
        || u64::from(metrics.verified_route_runs) != runs
        || u64::from(metrics.verified_result_hashes) != runs
    {
        return Err(
            "recommended candidate does not have complete task/oracle/route/hash metrics".into(),
        );
    }
    for result in results {
        let evidence = &result.evidence;
        if !evidence.task_executed
            || !evidence.outcome_oracle_evaluated
            || !evidence.recommendation_eligible
            || !evidence.route_verified
            || !evidence.successful
            || evidence.oracle.as_ref().is_none_or(|oracle| !oracle.pass)
        {
            return Err("recommended result lacks executed/oracle/route success evidence".into());
        }
        if evidence.generated_by != "planr-instrumented-live-host-runner"
            || evidence.metrics_source != MetricsSource::TrustedTelemetry
            || evidence.metering_confidence != MeteringMode::Trusted
            || !route_is_trusted_telemetry(&evidence.route_observation)
        {
            return Err("recommended result lacks trusted telemetry evidence".into());
        }
    }
    require_json_string(
        &report["environment"],
        "runner",
        "planr-instrumented-live-host-runner",
    )?;
    if let Some(host) = requested_host
        && report
            .pointer("/environment/requested_host")
            .and_then(serde_json::Value::as_str)
            != Some(host)
    {
        return Err(format!(
            "recommended evaluation was not run for required host `{host}`"
        ));
    }
    let recommendation = report["recommended"]
        .as_array()
        .and_then(|recommendations| {
            recommendations.iter().find(|recommendation| {
                recommendation
                    .get("policy")
                    .and_then(serde_json::Value::as_str)
                    == Some(policy_id)
                    && recommendation
                        .get("binding")
                        .and_then(serde_json::Value::as_str)
                        == Some(binding_id)
            })
        })
        .ok_or_else(|| "recommended candidate is absent from report.recommended".to_string())?;
    require_json_string(recommendation, "status", "recommended")?;
    require_json_string(recommendation, "label", "recommended")?;
    require_json_string(recommendation, "fixture_id", &suite.id)?;
    require_json_string(recommendation, "fixture_version", &suite.version)?;
    require_json_string(
        recommendation,
        "fixture_sha256",
        &sha256(SUITE_SOURCE.as_bytes()),
    )?;
    let expected_hashes = results
        .iter()
        .map(|result| serde_json::Value::String(result.result_sha256.clone()))
        .collect::<Vec<_>>();
    if recommendation.get("result_hashes") != Some(&serde_json::Value::Array(expected_hashes)) {
        return Err("recommendation result_hashes do not bind the candidate evidence".into());
    }
    Ok(())
}

fn route_is_trusted_telemetry(observation: &RouteObservation) -> bool {
    let trusted_string = |dimension: &StringDimension| {
        dimension.enforcement == EnforcementState::Verified
            && dimension.evidence == Some(EvidenceSource::TelemetryReceipt)
            && dimension.value.is_some()
    };
    effective_route_matches_requested(observation)
        && trusted_string(&observation.effective.agent_type)
        && trusted_string(&observation.effective.model)
        && trusted_string(&observation.effective.effort)
        && observation.effective.context_fork.enforcement == EnforcementState::Verified
        && observation.effective.context_fork.evidence == Some(EvidenceSource::TelemetryReceipt)
        && observation.effective.context_fork.value.is_some()
        && observation.metering.tool_calls.confidence == MeteringMode::Trusted
        && observation.metering.tokens.confidence == MeteringMode::Trusted
        && observation.metering.credits_micros.confidence == MeteringMode::Trusted
}

fn require_json_string(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let observed = value.get(field).and_then(serde_json::Value::as_str);
    if observed == Some(expected) {
        Ok(())
    } else {
        Err(format!(
            "preset evaluation `{field}` mismatch: expected `{expected}`, observed `{}`",
            observed.unwrap_or("<missing>")
        ))
    }
}

fn json_u64(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("preset evaluation `{field}` must be an unsigned integer"))
}

fn require_json_u64(value: &serde_json::Value, field: &str, expected: u64) -> Result<(), String> {
    let observed = json_u64(value, field)?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "preset evaluation `{field}` mismatch: expected {expected}, observed {observed}"
        ))
    }
}

fn evaluate_embedded_suite_with_clock(
    options: &EvaluationOptions,
    now_unix: impl FnOnce() -> u64,
) -> Result<PresetEvaluationReport, String> {
    let process_adapter = options
        .live_host
        .as_ref()
        .map(|command| ProcessLiveHostAdapter { command });
    evaluate_embedded_suite_with_adapter(
        options,
        now_unix,
        process_adapter
            .as_ref()
            .map(|adapter| adapter as &dyn LiveHostAdapter),
    )
}

fn evaluate_embedded_suite_with_adapter(
    options: &EvaluationOptions,
    now_unix: impl FnOnce() -> u64,
    live_host: Option<&dyn LiveHostAdapter>,
) -> Result<PresetEvaluationReport, String> {
    let suite: EvaluationSuite = toml::from_str(SUITE_SOURCE)
        .map_err(|error| format!("preset evaluation fixture parse failed: {error}"))?;
    validate_suite(&suite)?;
    let fixture_sha256 = sha256(SUITE_SOURCE.as_bytes());
    let evaluated_at_unix = options.at_unix.unwrap_or_else(now_unix);
    if options.trusted_telemetry.is_some() && live_host.is_none() {
        return Err("trusted telemetry requires an explicit live host adapter".to_string());
    }
    let suite_reference = VersionReference {
        id: suite.id.clone(),
        version: suite.version.clone(),
    };
    let trusted_telemetry = options
        .trusted_telemetry
        .as_ref()
        .map(VerifiedTelemetryCollector::load)
        .transpose()?;
    let telemetry_run_id = trusted_telemetry
        .as_ref()
        .map(|_| uuid::Uuid::new_v4().to_string());
    let codex_dispatch_contract = evaluate_sol_terra_luna_contract()?;
    let task_fixtures = suite
        .tasks
        .iter()
        .map(|task| {
            Ok(TaskProvenance {
                id: task.id.clone(),
                version: task.version.clone(),
                kind: task.kind,
                input_sha256: hash_json(task)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut candidate_ids = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut transition_counts = BTreeMap::new();
    for fixture in &suite.candidates {
        if !candidate_ids.insert(&fixture.id) {
            return Err(format!("duplicate evaluation candidate `{}`", fixture.id));
        }
        let report = evaluate_candidate(
            &suite_reference,
            fixture,
            &suite.tasks,
            &suite.thresholds,
            evaluated_at_unix,
            suite.expires_at_unix,
            options.host.as_deref(),
            codex_dispatch_contract.pass,
            live_host,
            trusted_telemetry.as_ref(),
            telemetry_run_id.as_deref(),
        )?;
        for result in &report.results {
            let transition = &result.evidence.transition;
            let count = transition_counts
                .entry(transition.clone())
                .or_insert_with(TransitionCount::default);
            count.attempted += 1;
            if result.evidence.transition_allowed {
                count.allowed += 1;
            } else {
                count.rejected += 1;
            }
        }
        candidates.push(report);
    }

    let recommendations = candidates
        .iter()
        .filter(|candidate| candidate.status == EvaluationStatus::Recommended)
        .map(|candidate| Recommendation {
            policy: candidate.policy.id.clone(),
            binding: candidate.binding.id.clone(),
            status: candidate.status,
            label: "recommended".to_string(),
            fixture_id: suite.id.clone(),
            fixture_version: suite.version.clone(),
            fixture_sha256: fixture_sha256.clone(),
            result_hashes: candidate
                .results
                .iter()
                .map(|result| result.result_sha256.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let reproducible_evidence = candidates
        .iter()
        .all(|candidate| candidate.evidence_complete)
        && codex_dispatch_contract.pass;

    Ok(PresetEvaluationReport {
        schema_version: 1,
        suite: SuiteProvenance {
            id: suite.id,
            version: suite.version,
            verified_at_unix: suite.verified_at_unix,
            expires_at_unix: suite.expires_at_unix,
            evaluated_at_unix,
            fixture_sha256,
            sol_terra_luna_fixture_sha256: sha256(SOL_TERRA_LUNA_SOURCE.as_bytes()),
        },
        environment: EnvironmentProvenance {
            runner: if trusted_telemetry.is_some() {
                "planr-instrumented-live-host-runner"
            } else if live_host.is_some() {
                "planr-live-host-runner"
            } else {
                "planr-offline-policy-simulator"
            }
            .to_string(),
            runner_schema_version: 1,
            planr_version: env!("CARGO_PKG_VERSION").to_string(),
            operating_system: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            requested_host: options.host.clone(),
        },
        thresholds: suite.thresholds,
        task_fixtures,
        candidates,
        transition_counts,
        codex_dispatch_contract,
        recommended: recommendations,
        reproducible_evidence,
    })
}

fn validate_suite(suite: &EvaluationSuite) -> Result<(), String> {
    if suite.schema_version != 1 {
        return Err(format!(
            "preset evaluation fixture schema_version {} is unsupported",
            suite.schema_version
        ));
    }
    if suite.id.trim().is_empty() || suite.version.trim().is_empty() {
        return Err("preset evaluation fixture id/version must not be blank".to_string());
    }
    if suite.expires_at_unix <= suite.verified_at_unix {
        return Err("preset evaluation expiry must follow verification time".to_string());
    }
    if suite.candidates.is_empty() || suite.tasks.is_empty() {
        return Err("preset evaluation fixture must contain candidates and tasks".to_string());
    }
    if suite.tasks.iter().any(|task| {
        task.input.trim().is_empty()
            || task.artifact_kind.trim().is_empty()
            || task.expected_output.trim().is_empty()
            || task.objective.trim().is_empty()
    }) {
        return Err(
            "preset evaluation tasks require non-empty objective, input, artifact kind, and output oracle"
                .to_string(),
        );
    }
    let kinds = suite
        .tasks
        .iter()
        .map(|task| task.kind as u8)
        .collect::<BTreeSet<_>>();
    if kinds.len() != 7 {
        return Err("preset evaluation fixture must cover all seven task kinds".to_string());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn evaluate_candidate(
    suite: &VersionReference,
    fixture: &CandidateFixture,
    tasks: &[TaskFixture],
    thresholds: &EvaluationThresholds,
    evaluated_at_unix: u64,
    expires_at_unix: u64,
    requested_host: Option<&str>,
    codex_contract_pass: bool,
    live_host: Option<&dyn LiveHostAdapter>,
    trusted_telemetry: Option<&VerifiedTelemetryCollector>,
    telemetry_run_id: Option<&str>,
) -> Result<CandidateReport, String> {
    let policy_source = builtin_policy(Path::new(&fixture.policy))
        .ok_or_else(|| format!("unknown built-in policy `{}`", fixture.policy))?;
    let binding_source = builtin_binding(Path::new(&fixture.binding))
        .ok_or_else(|| format!("unknown built-in binding `{}`", fixture.binding))?;
    let policy = parse_policy(policy_source.content)
        .map_err(|error| format!("{}: {error}", fixture.policy))?;
    let binding = parse_host_binding(binding_source.content)
        .map_err(|error| format!("{}: {error}", fixture.binding))?;
    let composed = compose_preset(&policy, &binding, binding.verification.verified_at_unix);
    let composition_ok = composed.compatibility.ok;
    let host_compatible = requested_host.is_none_or(|host| binding_supports_host(&binding, host));

    let mut results = Vec::new();
    for task in tasks {
        results.push(if let Some(adapter) = live_host {
            generate_live_result(
                suite,
                fixture,
                task,
                &policy,
                &binding,
                composition_ok,
                adapter,
                trusted_telemetry,
                telemetry_run_id,
                evaluated_at_unix,
            )?
        } else {
            generate_result(fixture, task, &policy, &binding, composition_ok)?
        });
    }
    let metrics = aggregate_metrics(&results);
    let threshold_results = threshold_results(&metrics, thresholds);
    let generated_evidence_complete = composition_ok
        && codex_contract_pass
        && results.iter().all(result_hash_valid)
        && metrics.transition_contract_failures == 0;
    let live_artifact_evidence_complete = live_host.is_none()
        || (metrics.actual_task_runs == metrics.runs
            && metrics.oracle_passes == metrics.runs
            && results.iter().all(|result| {
                result.evidence.task_executed
                    && result.evidence.outcome_oracle_evaluated
                    && result
                        .evidence
                        .oracle
                        .as_ref()
                        .is_some_and(|oracle| oracle.pass)
            }));
    let evidence_complete = generated_evidence_complete && live_artifact_evidence_complete;
    let observed_host = results
        .first()
        .and_then(|result| result.evidence.host.clone());
    let consistent_live_host = observed_host.as_ref().is_some_and(|expected| {
        results
            .iter()
            .all(|result| result.evidence.host.as_ref() == Some(expected))
    });
    let recommendation_evidence_complete = live_host.is_some()
        && consistent_live_host
        && metrics.actual_task_runs == metrics.runs
        && metrics.oracle_passes == metrics.runs
        && metrics.verified_route_runs == metrics.runs
        && results
            .iter()
            .all(|result| result.evidence.recommendation_eligible);
    let thresholds_pass = threshold_results.iter().all(|result| result.pass);
    let (status, status_reasons) = evaluation_status(
        evidence_complete,
        recommendation_evidence_complete,
        thresholds_pass,
        host_compatible,
        live_host.is_some(),
        evaluated_at_unix,
        expires_at_unix,
    );
    let label = (status == EvaluationStatus::Recommended).then(|| "recommended".to_string());

    Ok(CandidateReport {
        id: fixture.id.clone(),
        policy: VersionReference {
            id: policy.id,
            version: policy.version,
        },
        binding: VersionReference {
            id: binding.id.clone(),
            version: binding.version.clone(),
        },
        model_versions: model_provenance(&binding),
        live_host: observed_host,
        status,
        status_reasons,
        metrics,
        threshold_results,
        evidence_scope: if live_host.is_some() {
            EvidenceScope::LiveTaskRun
        } else {
            EvidenceScope::PolicySimulation
        },
        evidence_complete,
        recommendation_evidence_complete,
        label,
        results,
    })
}

fn binding_supports_host(binding: &HostBindingV1, host: &str) -> bool {
    binding.host == host
        || (binding.host == "mixed-host"
            && binding
                .profiles
                .values()
                .any(|profile| profile.client == host))
}

fn evaluation_status(
    evidence_complete: bool,
    recommendation_evidence_complete: bool,
    thresholds_pass: bool,
    host_compatible: bool,
    live_evidence: bool,
    evaluated_at_unix: u64,
    expires_at_unix: u64,
) -> (EvaluationStatus, Vec<String>) {
    if !host_compatible {
        return (
            EvaluationStatus::Incompatible,
            vec!["requested host is incompatible with the evaluated binding".to_string()],
        );
    }
    if evaluated_at_unix > expires_at_unix {
        return (
            EvaluationStatus::Stale,
            vec![format!(
                "evaluation evidence expired at unix {expires_at_unix}"
            )],
        );
    }
    if !evidence_complete {
        return (
            EvaluationStatus::Unverified,
            vec![if live_evidence {
                "live execution was attempted, but one or more required challenge-bound task artifacts or outcome oracles are missing or invalid"
                    .to_string()
            } else {
                "generated evidence or result hashes are incomplete".to_string()
            }],
        );
    }
    if recommendation_evidence_complete && thresholds_pass {
        (
            EvaluationStatus::Recommended,
            vec![
                "executed task evidence and outcome oracles satisfy every recommendation threshold"
                    .to_string(),
            ],
        )
    } else {
        (
            EvaluationStatus::Verified,
            vec![if recommendation_evidence_complete {
                "executed task evidence is verified but one or more recommendation thresholds failed"
                    .to_string()
            } else if live_evidence {
                "live task artifacts were evaluated, but this process-adapter boundary has no independently trusted route or cost instrumentation"
                    .to_string()
            } else {
                "offline policy simulation verified; no task workflow, outcome oracle, or effective host route was observed"
                    .to_string()
            }],
        )
    }
}

fn generate_result(
    candidate: &CandidateFixture,
    task: &TaskFixture,
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    composition_ok: bool,
) -> Result<ResultArtifact, String> {
    let input_sha256 = hash_json(task)?;
    let request = transition_request(task.transition);
    let projected_metrics = projected_metrics(policy, task);
    let state = transition_state(policy, &projected_metrics);
    let decision = resolve_transition(policy, &state, &request);
    let transition_allowed = decision.is_allowed();
    let transition_contract_pass = !matches!(
        decision,
        TransitionDecision::Rejected {
            reason: TransitionRejectionReason::InvalidPolicy,
            ..
        }
    );
    let route = route_observation(
        policy,
        binding,
        task.transition,
        &task.id,
        &projected_metrics,
        false,
    )?;
    let route_verified = validate_route_observation(&route).is_ok() && route_is_verified(&route);
    let worker_permissions = policy.execution.roles.get("worker");
    let tool_budget = policy.usage.max_tool_calls.unwrap_or_default();
    let capacity_ok = tool_budget >= task.minimum_tool_budget;
    let write_ok = !task.requires_write
        || worker_permissions
            .is_some_and(|permissions| !permissions.filesystem.write_roots.is_empty());
    let safety_ok = !matches!(task.kind, TaskKind::Security) || transition_allowed;
    let checks = vec![
        generated_check(
            "composition",
            composition_ok,
            "policy and binding compose through the production preset service",
        ),
        generated_check(
            "route_evidence",
            route_verified,
            "offline simulation has no effective host/process route evidence",
        ),
        generated_check(
            "transition_contract",
            transition_contract_pass,
            "transition resolved without an invalid-policy result",
        ),
        generated_check(
            "tool_budget",
            capacity_ok,
            &format!(
                "policy tool budget {tool_budget} >= {}",
                task.minimum_tool_budget
            ),
        ),
        generated_check(
            "write_capability",
            write_ok,
            if task.requires_write {
                "task requires a declared worker write root"
            } else {
                "task is read-only"
            },
        ),
        generated_check(
            "safety_stop",
            safety_ok,
            "security tasks must resolve to an allowed safety stop",
        ),
    ];
    let policy_checks = checks
        .iter()
        .filter(|check| check.name != "route_evidence")
        .collect::<Vec<_>>();
    let passed = policy_checks.iter().filter(|check| check.pass).count() as u32;
    let policy_check_score_bps = passed * 10_000 / policy_checks.len() as u32;
    let simulation_passed = policy_checks.iter().all(|check| check.pass);
    let evidence = EvaluationEvidence {
        schema_version: 1,
        generated_by: "planr-offline-policy-simulator".to_string(),
        evidence_scope: EvidenceScope::PolicySimulation,
        task_executed: false,
        outcome_oracle_evaluated: false,
        recommendation_eligible: false,
        host: None,
        oracle: None,
        candidate_id: candidate.id.clone(),
        task_id: task.id.clone(),
        task_kind: task.kind,
        checks,
        quality_score_bps: policy_check_score_bps,
        successful: simulation_passed,
        metrics_source: MetricsSource::EstimatedProjection,
        metering_confidence: MeteringMode::Estimated,
        tool_calls: projected_metrics.tool_calls,
        tokens: projected_metrics.tokens,
        credits_micros: projected_metrics.credits_micros,
        latency_ms: projected_metrics.latency_ms,
        retries: 0,
        availability_fallbacks: 0,
        quality_escalations: 0,
        corrections: 0,
        violations: 0,
        transition: transition_name(task.transition).to_string(),
        transition_allowed,
        transition_contract_pass,
        route_verified,
        route_observation: route,
    };
    let result_sha256 = hash_json(&evidence)?;
    Ok(ResultArtifact {
        task: VersionReference {
            id: task.id.clone(),
            version: task.version.clone(),
        },
        task_kind: task.kind,
        input_sha256,
        result_sha256,
        evidence,
    })
}

#[allow(clippy::too_many_arguments)]
fn generate_live_result(
    suite: &VersionReference,
    candidate: &CandidateFixture,
    task: &TaskFixture,
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    composition_ok: bool,
    adapter: &dyn LiveHostAdapter,
    trusted_telemetry: Option<&VerifiedTelemetryCollector>,
    telemetry_run_id: Option<&str>,
    evaluated_at_unix: u64,
) -> Result<ResultArtifact, String> {
    let worker = binding
        .profiles
        .get("worker")
        .ok_or_else(|| format!("binding `{}` has no worker profile", binding.id))?;
    let input_sha256 = hash_json(task)?;
    let workspace = LiveExecutionWorkspace::create(
        &candidate.id,
        &task.id,
        &input_sha256,
        &task.artifact_kind,
    )?;
    let request = LiveHostRequest {
        schema_version: 1,
        suite: suite.clone(),
        candidate: VersionReference {
            id: candidate.id.clone(),
            version: suite.version.clone(),
        },
        policy: VersionReference {
            id: policy.id.clone(),
            version: policy.version.clone(),
        },
        binding: VersionReference {
            id: binding.id.clone(),
            version: binding.version.clone(),
        },
        task: VersionReference {
            id: task.id.clone(),
            version: task.version.clone(),
        },
        task_kind: task.kind,
        requested_route: requested_route("worker", worker),
        input: task.input.clone(),
        input_sha256: input_sha256.clone(),
        artifact_kind: task.artifact_kind.clone(),
        workspace_path: workspace.root.display().to_string(),
        challenge_path: workspace.challenge_path.display().to_string(),
        artifact_path: workspace.artifact_path.display().to_string(),
    };
    let live = adapter.run(&request)?;
    let artifact = workspace.read_artifact();
    let trusted_telemetry = trusted_telemetry.and_then(|collector| {
        let (_, artifact_sha256) = artifact.as_ref()?;
        let run_id = telemetry_run_id?;
        collector.collect(&TelemetryCollectorRequest {
            schema_version: 1,
            run_id: run_id.to_string(),
            evaluated_at_unix,
            suite_id: suite.id.clone(),
            suite_version: suite.version.clone(),
            candidate_id: candidate.id.clone(),
            task_id: task.id.clone(),
            input_sha256: input_sha256.clone(),
            artifact_kind: task.artifact_kind.clone(),
            artifact_sha256: artifact_sha256.clone(),
            challenge_nonce: workspace.challenge_nonce.clone(),
            requested_route: request.requested_route.clone(),
        })
    });
    let telemetry_bound = trusted_telemetry.as_ref().is_some_and(|receipt| {
        receipt.host_id == live.response.host_id
            && receipt.host_version == live.response.host_version
    });
    let trusted_telemetry = telemetry_bound
        .then_some(trusted_telemetry.as_ref())
        .flatten();
    let tool_calls = trusted_telemetry.map_or(live.response.tool_calls, |value| value.tool_calls);
    let tokens = trusted_telemetry.map_or(live.response.tokens, |value| value.tokens);
    let credits_micros =
        trusted_telemetry.map_or(live.response.credits_micros, |value| value.credits_micros);
    let retries = trusted_telemetry.map_or(live.response.retries, |value| value.retries);
    let availability_fallbacks = trusted_telemetry
        .map_or(live.response.availability_fallbacks, |value| {
            value.availability_fallbacks
        });
    let quality_escalations = trusted_telemetry
        .map_or(live.response.quality_escalations, |value| {
            value.quality_escalations
        });
    let corrections =
        trusted_telemetry.map_or(live.response.corrections, |value| value.corrections);
    let violations = trusted_telemetry.map_or(live.response.violations, |value| value.violations);
    let effective_model = trusted_telemetry.map_or_else(
        || live.response.effective_model.clone(),
        |value| value.effective_model.clone(),
    );
    let effective_effort = trusted_telemetry.map_or_else(
        || live.response.effective_effort.clone(),
        |value| value.effective_effort.clone(),
    );
    let effective_context_fork = trusted_telemetry.map_or_else(
        || live.response.effective_context_fork.clone(),
        |value| value.effective_context_fork.clone(),
    );
    let effective_agent_type = trusted_telemetry.map_or_else(
        || live.response.effective_agent_type.clone(),
        |value| value.effective_agent_type.clone(),
    );
    let effective_role = trusted_telemetry.map_or_else(
        || live.response.effective_role.clone(),
        |value| value.effective_role.clone(),
    );
    let thread_id = trusted_telemetry.map_or_else(
        || live.response.thread_id.clone(),
        |value| value.thread_id.clone(),
    );
    let status = trusted_telemetry.map_or_else(
        || live.response.status.clone(),
        |value| value.status.clone(),
    );
    let observed = ProjectedMetrics {
        tool_calls,
        tokens,
        credits_micros,
        latency_ms: live.latency_ms,
    };
    let transition_request = transition_request(task.transition);
    let state = transition_state(policy, &observed);
    let decision = resolve_transition(policy, &state, &transition_request);
    let transition_allowed = decision.is_allowed();
    let transition_observed = match task.transition {
        FixtureTransition::Retry => retries > 0,
        FixtureTransition::AvailabilityFallback => availability_fallbacks > 0,
        FixtureTransition::QualityEscalation => quality_escalations > 0,
        FixtureTransition::QuotaDowngrade => true,
        FixtureTransition::SafetyStop => transition_allowed,
    };
    let transition_contract_pass = transition_observed
        && !matches!(
            decision,
            TransitionDecision::Rejected {
                reason: TransitionRejectionReason::InvalidPolicy,
                ..
            }
        );
    let route = live_route_observation(
        policy,
        binding,
        task.transition,
        &task.id,
        &observed,
        &effective_model,
        effective_effort.as_deref(),
        effective_context_fork.clone(),
        &effective_agent_type,
        effective_role.as_deref(),
        thread_id.as_deref(),
        status.as_deref(),
        trusted_telemetry.is_some(),
    )?;
    let expected_fork = worker.fork_turns.clone().unwrap_or(ContextForkMode::None);
    let request_bound = live.response.candidate_id == candidate.id
        && live.response.task_id == task.id
        && live.response.input_sha256 == input_sha256;
    let artifact_kind_matches = live.response.artifact_kind == task.artifact_kind;
    let effective_matches_binding = effective_model == worker.model
        && effective_effort == worker.effort
        && effective_context_fork == expected_fork
        && worker.agent_type.as_deref() == Some(effective_agent_type.as_str());
    let route_claim_well_formed = request_bound
        && effective_matches_binding
        && effective_route_matches_requested(&route)
        && validate_route_observation(&route).is_ok();
    let route_verified = route_claim_well_formed && route_is_verified(&route);
    let expected_output = format!("{}:{}:{}", candidate.id, task.id, task.expected_output);
    let expected_sha256 = sha256(expected_output.as_bytes());
    let artifact_schema_valid = artifact
        .as_ref()
        .is_some_and(|(artifact, _)| artifact.schema_version == 1);
    let artifact_hash_matches = artifact.as_ref().is_some_and(|(_, digest)| {
        !live.response.artifact_sha256.is_empty() && *digest == live.response.artifact_sha256
    });
    let artifact_request_bound = artifact.as_ref().is_some_and(|(artifact, _)| {
        artifact.candidate_id == candidate.id
            && artifact.task_id == task.id
            && artifact.input_sha256 == input_sha256
            && artifact.artifact_kind == task.artifact_kind
    });
    let challenge_bound = artifact
        .as_ref()
        .is_some_and(|(artifact, _)| artifact.challenge_sha256 == workspace.challenge_sha256);
    let artifact_output = artifact
        .as_ref()
        .map_or("", |(artifact, _)| artifact.output.as_str());
    let response_matches_artifact = artifact.is_some() && live.response.output == artifact_output;
    let actual_sha256 = sha256(artifact_output.as_bytes());
    let oracle_pass = artifact_schema_valid
        && artifact_hash_matches
        && artifact_request_bound
        && challenge_bound
        && artifact_output == expected_output;
    let oracle = OracleEvidence {
        kind: "planr_workspace_artifact".to_string(),
        expected_sha256,
        actual_sha256,
        pass: oracle_pass,
    };
    let tool_budget_ok =
        policy.usage.max_tool_calls.unwrap_or_default() >= task.minimum_tool_budget;
    let write_capability_ok = !task.requires_write
        || policy
            .execution
            .roles
            .get("worker")
            .is_some_and(|permissions| !permissions.filesystem.write_roots.is_empty());
    let trusted_metering = trusted_telemetry.is_some();
    let checks = vec![
        generated_check(
            "composition",
            composition_ok,
            "policy and binding compose through the production preset service",
        ),
        generated_check(
            "request_binding",
            request_bound,
            "the process response echoes the candidate, task, and immutable input hash",
        ),
        generated_check(
            "artifact_kind",
            artifact_kind_matches,
            "the artifact kind matches the task-specific fixture contract",
        ),
        generated_check(
            "artifact_file",
            artifact_schema_valid,
            "the adapter wrote a bounded schema-v1 artifact at Planr's workspace path",
        ),
        generated_check(
            "artifact_hash",
            artifact_hash_matches,
            "Planr's hash of the workspace artifact matches the response digest",
        ),
        generated_check(
            "artifact_request_binding",
            artifact_request_bound,
            "the workspace artifact binds candidate, task, input hash, and artifact kind",
        ),
        generated_check(
            "challenge_binding",
            challenge_bound,
            "the workspace artifact proves the adapter read Planr-controlled challenge state",
        ),
        generated_check(
            "response_artifact_consistency",
            response_matches_artifact,
            "the process response and Planr-read artifact report the same task output",
        ),
        generated_check(
            "task_output_oracle",
            oracle_pass,
            "the schema-checked workspace artifact satisfies the versioned task oracle",
        ),
        generated_check(
            "trusted_telemetry",
            trusted_metering,
            "route and usage evidence is bound to a verified Ed25519 telemetry receipt",
        ),
        generated_check(
            "route_evidence",
            route_verified,
            "effective route evidence matches the binding and comes from trusted telemetry",
        ),
        generated_check(
            "trusted_metering",
            trusted_metering,
            "tool, token, and credit measurements come from trusted telemetry",
        ),
        generated_check(
            "transition_contract",
            transition_contract_pass,
            "the task's expected transition was observed and accepted by policy",
        ),
        generated_check(
            "policy_violations",
            violations == 0,
            "trusted telemetry or the untrusted host report contains no policy violations",
        ),
        generated_check(
            "tool_budget",
            tool_budget_ok,
            "the candidate policy has enough tool budget for this fixture",
        ),
        generated_check(
            "write_capability",
            write_capability_ok,
            "write-requiring fixtures require a declared worker write root",
        ),
        generated_check(
            "safety_stop",
            !matches!(task.kind, TaskKind::Security) || transition_allowed,
            "security tasks must resolve to an allowed safety stop",
        ),
    ];
    let passed = checks.iter().filter(|check| check.pass).count() as u32;
    let quality_score_bps = passed * 10_000 / checks.len() as u32;
    let successful = composition_ok
        && request_bound
        && artifact_kind_matches
        && oracle_pass
        && transition_contract_pass
        && violations == 0
        && tool_budget_ok
        && write_capability_ok
        && (!matches!(task.kind, TaskKind::Security) || transition_allowed);
    let task_executed = request_bound
        && artifact_kind_matches
        && artifact_schema_valid
        && artifact_hash_matches
        && artifact_request_bound
        && challenge_bound
        && response_matches_artifact
        && oracle_pass
        && tool_budget_ok
        && write_capability_ok;
    let evidence = EvaluationEvidence {
        schema_version: 1,
        generated_by: if trusted_metering {
            "planr-instrumented-live-host-runner"
        } else {
            "planr-live-host-runner"
        }
        .to_string(),
        evidence_scope: EvidenceScope::LiveTaskRun,
        task_executed,
        outcome_oracle_evaluated: artifact_schema_valid,
        recommendation_eligible: successful && route_verified && trusted_metering,
        host: Some(VersionReference {
            id: live.response.host_id.clone(),
            version: live.response.host_version.clone(),
        }),
        oracle: Some(oracle),
        candidate_id: candidate.id.clone(),
        task_id: task.id.clone(),
        task_kind: task.kind,
        checks,
        quality_score_bps,
        successful,
        metrics_source: if trusted_metering {
            MetricsSource::TrustedTelemetry
        } else {
            MetricsSource::HostReported
        },
        metering_confidence: if trusted_metering {
            MeteringMode::Trusted
        } else {
            MeteringMode::Estimated
        },
        tool_calls,
        tokens,
        credits_micros,
        latency_ms: live.latency_ms,
        retries,
        availability_fallbacks,
        quality_escalations,
        corrections,
        violations,
        transition: transition_name(task.transition).to_string(),
        transition_allowed,
        transition_contract_pass,
        route_verified,
        route_observation: route,
    };
    let result_sha256 = hash_json(&evidence)?;
    Ok(ResultArtifact {
        task: VersionReference {
            id: task.id.clone(),
            version: task.version.clone(),
        },
        task_kind: task.kind,
        input_sha256,
        result_sha256,
        evidence,
    })
}

fn generated_check(name: &str, pass: bool, detail: &str) -> GeneratedCheck {
    GeneratedCheck {
        name: name.to_string(),
        pass,
        detail: detail.to_string(),
    }
}

#[derive(Clone, Copy)]
struct ProjectedMetrics {
    tool_calls: u64,
    tokens: u64,
    credits_micros: u64,
    latency_ms: u64,
}

fn projected_metrics(policy: &UsagePolicyV1, task: &TaskFixture) -> ProjectedMetrics {
    let intensity = u64::from(policy.usage.max_active_agents);
    let projected_calls = task.work_units.saturating_mul(intensity);
    let tool_calls = policy
        .usage
        .max_tool_calls
        .map_or(projected_calls, |limit| projected_calls.min(limit));
    ProjectedMetrics {
        tool_calls,
        tokens: tool_calls.saturating_mul(250),
        credits_micros: tool_calls.saturating_mul(30_000),
        latency_ms: task
            .work_units
            .saturating_mul(intensity)
            .saturating_mul(100),
    }
}

fn aggregate_metrics(results: &[ResultArtifact]) -> CandidateMetrics {
    let count = results.len() as u64;
    let successful = results
        .iter()
        .filter(|result| result.evidence.successful)
        .count() as u64;
    let total_credits = results
        .iter()
        .map(|result| result.evidence.credits_micros)
        .sum::<u64>();
    let mut latencies = results
        .iter()
        .map(|result| result.evidence.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let p95_index = (latencies.len() * 95).div_ceil(100).saturating_sub(1);
    let safety_results = results
        .iter()
        .filter(|result| result.evidence.transition == "safety_stop")
        .collect::<Vec<_>>();
    CandidateMetrics {
        metrics_source: results[0].evidence.metrics_source,
        runs: results.len() as u32,
        actual_task_runs: results
            .iter()
            .filter(|result| result.evidence.task_executed)
            .count() as u32,
        oracle_passes: results
            .iter()
            .filter(|result| {
                result
                    .evidence
                    .oracle
                    .as_ref()
                    .is_some_and(|oracle| oracle.pass)
            })
            .count() as u32,
        average_quality_score_bps: (results
            .iter()
            .map(|result| u64::from(result.evidence.quality_score_bps))
            .sum::<u64>()
            / count) as u32,
        reliability_bps: ((successful * 10_000) / count) as u32,
        total_tool_calls: results
            .iter()
            .map(|result| result.evidence.tool_calls)
            .sum(),
        total_tokens: results.iter().map(|result| result.evidence.tokens).sum(),
        total_credits_micros: total_credits,
        average_credits_micros: total_credits / count,
        p95_latency_ms: latencies[p95_index],
        retries: results.iter().map(|result| result.evidence.retries).sum(),
        availability_fallbacks: results
            .iter()
            .map(|result| result.evidence.availability_fallbacks)
            .sum(),
        quality_escalations: results
            .iter()
            .map(|result| result.evidence.quality_escalations)
            .sum(),
        corrections: results
            .iter()
            .map(|result| result.evidence.corrections)
            .sum(),
        violations: results
            .iter()
            .map(|result| result.evidence.violations)
            .sum(),
        transition_contract_failures: results
            .iter()
            .filter(|result| !result.evidence.transition_contract_pass)
            .count() as u32,
        safety_stop_attempts: safety_results.len() as u32,
        safety_stop_failures: safety_results
            .iter()
            .filter(|result| !result.evidence.transition_allowed)
            .count() as u32,
        verified_route_runs: results
            .iter()
            .filter(|result| result.evidence.route_verified)
            .count() as u32,
        verified_result_hashes: results
            .iter()
            .filter(|result| result_hash_valid(result))
            .count() as u32,
    }
}

fn result_hash_valid(result: &ResultArtifact) -> bool {
    hash_json(&result.evidence).is_ok_and(|hash| hash == result.result_sha256)
}

fn threshold_results(
    metrics: &CandidateMetrics,
    thresholds: &EvaluationThresholds,
) -> Vec<ThresholdResult> {
    vec![
        threshold(
            "minimum_runs",
            format!(">= {}", thresholds.minimum_runs),
            metrics.runs,
            metrics.runs >= thresholds.minimum_runs,
        ),
        threshold(
            "quality",
            format!(">= {} bps", thresholds.minimum_quality_score_bps),
            metrics.average_quality_score_bps,
            metrics.average_quality_score_bps >= thresholds.minimum_quality_score_bps,
        ),
        threshold(
            "reliability",
            format!(">= {} bps", thresholds.minimum_reliability_bps),
            metrics.reliability_bps,
            metrics.reliability_bps >= thresholds.minimum_reliability_bps,
        ),
        threshold(
            "average_cost",
            format!(
                "<= {} credits_micros",
                thresholds.maximum_average_credits_micros
            ),
            metrics.average_credits_micros,
            metrics.average_credits_micros <= thresholds.maximum_average_credits_micros,
        ),
        threshold(
            "p95_latency",
            format!("<= {} ms", thresholds.maximum_p95_latency_ms),
            metrics.p95_latency_ms,
            metrics.p95_latency_ms <= thresholds.maximum_p95_latency_ms,
        ),
        threshold(
            "transition_contract",
            format!(
                "<= {} failures",
                thresholds.maximum_transition_contract_failures
            ),
            metrics.transition_contract_failures,
            metrics.transition_contract_failures <= thresholds.maximum_transition_contract_failures,
        ),
        threshold(
            "safety_stop",
            format!("<= {} failures", thresholds.maximum_safety_stop_failures),
            metrics.safety_stop_failures,
            metrics.safety_stop_failures <= thresholds.maximum_safety_stop_failures,
        ),
        threshold(
            "actual_task_runs",
            "all fixtures executed".to_string(),
            format!("{}/{}", metrics.actual_task_runs, metrics.runs),
            metrics.actual_task_runs == metrics.runs,
        ),
        threshold(
            "output_oracles",
            "all task output oracles pass".to_string(),
            format!("{}/{}", metrics.oracle_passes, metrics.runs),
            metrics.oracle_passes == metrics.runs,
        ),
        threshold(
            "route_evidence",
            if thresholds.require_verified_routes {
                "all runs verified".to_string()
            } else {
                "not required".to_string()
            },
            format!("{}/{}", metrics.verified_route_runs, metrics.runs),
            !thresholds.require_verified_routes || metrics.verified_route_runs == metrics.runs,
        ),
        threshold(
            "result_hashes",
            if thresholds.require_result_hashes {
                "all result hashes verified".to_string()
            } else {
                "not required".to_string()
            },
            format!("{}/{}", metrics.verified_result_hashes, metrics.runs),
            !thresholds.require_result_hashes || metrics.verified_result_hashes == metrics.runs,
        ),
    ]
}

fn threshold(name: &str, rule: String, actual: impl ToString, pass: bool) -> ThresholdResult {
    ThresholdResult {
        name: name.to_string(),
        rule,
        actual: actual.to_string(),
        pass,
    }
}

fn transition_request(transition: FixtureTransition) -> TransitionRequest {
    match transition {
        FixtureTransition::Retry => TransitionRequest::Retry {
            trigger: RetryTrigger::TransientToolFailure,
        },
        FixtureTransition::AvailabilityFallback => TransitionRequest::AvailabilityFallback {
            trigger: AvailabilityTrigger::ModelUnavailable,
            same_capability_class: true,
        },
        FixtureTransition::QualityEscalation => TransitionRequest::QualityEscalation {
            trigger: QualityTrigger::VerificationFailed,
            verification_evidence: true,
        },
        FixtureTransition::QuotaDowngrade => TransitionRequest::QuotaDowngrade {
            trigger: QuotaTrigger::BudgetThreshold,
            remaining_usage_evidence: true,
        },
        FixtureTransition::SafetyStop => TransitionRequest::SafetyStop {
            trigger: SafetyTrigger::UnsafeOperation,
            policy_rule: "fixture:unsafe-operation".to_string(),
        },
    }
}

fn transition_state(policy: &UsagePolicyV1, metrics: &ProjectedMetrics) -> TransitionState {
    TransitionState {
        current_depth: 1,
        projected_active_agents: 1,
        projected_parallel_readers: 1,
        projected_parallel_writers: 0,
        attempts_started: 0,
        same_route_retries: 0,
        availability_fallbacks: 0,
        quality_escalations: 0,
        quota_downgrades: 0,
        elapsed_seconds: metrics.latency_ms.div_ceil(1000),
        usage: UsageObservation {
            metering: policy.usage.metering,
            tool_calls: Some(metrics.tool_calls),
            tokens: Some(metrics.tokens),
            credits_micros: Some(metrics.credits_micros),
        },
        risk: RiskLevel::Low,
        material: false,
        review_work: false,
        pending_safety_stop: None,
    }
}

fn route_observation(
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    transition: FixtureTransition,
    task_id: &str,
    metrics: &ProjectedMetrics,
    process_evidence: bool,
) -> Result<RouteObservation, String> {
    let worker = binding
        .profiles
        .get("worker")
        .ok_or_else(|| format!("binding `{}` has no worker profile", binding.id))?;
    let fork = worker.fork_turns.clone().unwrap_or(ContextForkMode::None);
    Ok(route_observation_for(
        policy,
        binding,
        worker.profile.as_str(),
        worker.client.as_str(),
        worker.agent_type.as_deref(),
        worker.model.as_str(),
        worker.effort.as_deref(),
        fork,
        process_evidence,
        transition,
        task_id,
        metrics,
    ))
}

#[allow(clippy::too_many_arguments)]
fn live_route_observation(
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    transition: FixtureTransition,
    task_id: &str,
    metrics: &ProjectedMetrics,
    effective_model: &str,
    effective_effort: Option<&str>,
    effective_context_fork: ContextForkMode,
    effective_agent_type: &str,
    effective_role: Option<&str>,
    thread_id: Option<&str>,
    status: Option<&str>,
    trusted_telemetry: bool,
) -> Result<RouteObservation, String> {
    let worker = binding
        .profiles
        .get("worker")
        .ok_or_else(|| format!("binding `{}` has no worker profile", binding.id))?;
    let mut observation = route_observation(policy, binding, transition, task_id, metrics, false)?;
    let (enforcement, evidence, metering) = if trusted_telemetry {
        (
            EnforcementState::Verified,
            EvidenceSource::TelemetryReceipt,
            MeteringMode::Trusted,
        )
    } else {
        (
            EnforcementState::Estimated,
            EvidenceSource::HostReport,
            MeteringMode::Estimated,
        )
    };
    observation.effective = route_stage(
        effective_role,
        worker.profile.as_str(),
        worker.client.as_str(),
        Some(effective_agent_type),
        effective_model,
        effective_effort,
        effective_context_fork,
        enforcement,
        evidence,
        thread_id,
        status,
    );
    observation.transition.reason = format!("executed live task fixture {task_id}");
    observation.transition.evidence = vec![if trusted_telemetry {
        EvidenceSource::TelemetryReceipt
    } else {
        EvidenceSource::ProcessExit
    }];
    observation.metering = RouteMetering {
        wall_time_seconds: metered_with(metrics.latency_ms.div_ceil(1000), MeteringMode::Trusted),
        tool_calls: metered_with(metrics.tool_calls, metering),
        tokens: metered_with(metrics.tokens, metering),
        credits_micros: metered_with(metrics.credits_micros, metering),
    };
    Ok(observation)
}

#[allow(clippy::too_many_arguments)]
fn route_observation_for(
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    profile: &str,
    client: &str,
    agent_type: Option<&str>,
    model: &str,
    effort: Option<&str>,
    fork: ContextForkMode,
    process_evidence: bool,
    transition: FixtureTransition,
    task_id: &str,
    metrics: &ProjectedMetrics,
) -> RouteObservation {
    let requested = route_stage(
        Some("worker"),
        profile,
        client,
        agent_type,
        model,
        effort,
        fork.clone(),
        EnforcementState::RequestedOnly,
        EvidenceSource::Binding,
        None,
        None,
    );
    let resolved = route_stage(
        Some("worker"),
        profile,
        client,
        agent_type,
        model,
        effort,
        fork.clone(),
        EnforcementState::Verified,
        EvidenceSource::Binding,
        None,
        None,
    );
    let effective = if process_evidence {
        route_stage(
            Some("worker"),
            profile,
            client,
            agent_type,
            model,
            effort,
            fork,
            EnforcementState::Verified,
            EvidenceSource::ProcessExit,
            None,
            None,
        )
    } else {
        RouteStage {
            role: Some("worker".to_string()),
            profile: Some(profile.to_string()),
            client: Some(client.to_string()),
            agent_type: unavailable_string(),
            model: unavailable_string(),
            effort: unavailable_string(),
            context_fork: ForkDimension {
                value: None,
                enforcement: EnforcementState::Unavailable,
                evidence: None,
            },
            thread_id: None,
            status: None,
        }
    };
    RouteObservation {
        requested,
        resolved,
        effective,
        transition: RouteTransition {
            kind: route_transition(transition),
            reason: format!("offline policy simulation for task fixture {task_id}"),
            evidence: vec![EvidenceSource::LocalObservation],
        },
        policy: VersionReference {
            id: policy.id.clone(),
            version: policy.version.clone(),
        },
        binding: VersionReference {
            id: binding.id.clone(),
            version: binding.version.clone(),
        },
        metering: RouteMetering {
            wall_time_seconds: metered(metrics.latency_ms.div_ceil(1000)),
            tool_calls: metered(metrics.tool_calls),
            tokens: metered(metrics.tokens),
            credits_micros: metered(metrics.credits_micros),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn route_stage(
    role: Option<&str>,
    profile: &str,
    client: &str,
    agent_type: Option<&str>,
    model: &str,
    effort: Option<&str>,
    fork: ContextForkMode,
    enforcement: EnforcementState,
    evidence: EvidenceSource,
    thread_id: Option<&str>,
    status: Option<&str>,
) -> RouteStage {
    RouteStage {
        role: role.map(ToOwned::to_owned),
        profile: Some(profile.to_string()),
        client: Some(client.to_string()),
        agent_type: StringDimension {
            value: agent_type.map(ToOwned::to_owned),
            enforcement: if agent_type.is_some() {
                enforcement
            } else {
                EnforcementState::Unavailable
            },
            evidence: agent_type.map(|_| evidence),
        },
        model: StringDimension {
            value: Some(model.to_string()),
            enforcement,
            evidence: Some(evidence),
        },
        effort: StringDimension {
            value: effort.map(ToOwned::to_owned),
            enforcement: if effort.is_some() {
                enforcement
            } else {
                EnforcementState::Unavailable
            },
            evidence: effort.map(|_| evidence),
        },
        context_fork: ForkDimension {
            value: Some(fork),
            enforcement,
            evidence: Some(evidence),
        },
        thread_id: thread_id.map(ToOwned::to_owned),
        status: status.map(ToOwned::to_owned),
    }
}

fn unavailable_string() -> StringDimension {
    StringDimension {
        value: None,
        enforcement: EnforcementState::Unavailable,
        evidence: None,
    }
}

fn metered(value: u64) -> MeteredDimension {
    metered_with(value, MeteringMode::Estimated)
}

fn metered_with(value: u64, confidence: MeteringMode) -> MeteredDimension {
    MeteredDimension {
        value: Some(value),
        confidence,
    }
}

fn route_is_verified(observation: &RouteObservation) -> bool {
    effective_route_matches_requested(observation)
        && verified_string(&observation.effective.agent_type)
        && verified_string(&observation.effective.model)
        && verified_string(&observation.effective.effort)
        && matches!(
            observation.effective.context_fork,
            ForkDimension {
                value: Some(_),
                enforcement: EnforcementState::Verified,
                evidence: Some(
                    EvidenceSource::TelemetryReceipt
                        | EvidenceSource::ProcessExit
                        | EvidenceSource::LocalObservation
                )
            }
        )
}

fn verified_string(dimension: &StringDimension) -> bool {
    matches!(
        dimension,
        StringDimension {
            value: Some(_),
            enforcement: EnforcementState::Verified,
            evidence: Some(
                EvidenceSource::TelemetryReceipt
                    | EvidenceSource::ProcessExit
                    | EvidenceSource::LocalObservation
            )
        }
    )
}

fn evaluate_sol_terra_luna_contract() -> Result<CodexDispatchContract, String> {
    let policy_source = builtin_policy(Path::new("balanced"))
        .ok_or_else(|| "missing balanced policy".to_string())?;
    let policy = parse_policy(policy_source.content).map_err(|error| error.to_string())?;
    let binding = parse_host_binding(SOL_TERRA_LUNA_SOURCE)?;
    let mut errors = Vec::new();
    let composed = compose_preset(&policy, &binding, binding.verification.verified_at_unix);
    let none_fork_parameters_verified = composed.compatibility.ok
        && composed
            .dispatch
            .get("worker")
            .is_some_and(|dispatch| dispatch.fork_turns == ContextForkMode::None)
        && composed
            .registry
            .profiles
            .get("codex-terra-high")
            .is_some_and(|profile| {
                profile.model == "gpt-5.6-terra" && profile.effort.as_deref() == Some("high")
            });
    if !none_fork_parameters_verified {
        errors.push("none-fork Sol/Terra/Luna dispatch parameters did not compose".to_string());
    }
    let mut all_binding = binding.clone();
    all_binding.profiles.get_mut("worker").unwrap().fork_turns = Some(ContextForkMode::All);
    let all_composed = compose_preset(
        &policy,
        &all_binding,
        all_binding.verification.verified_at_unix,
    );
    let all_fork_rejected =
        !all_composed.compatibility.ok
            && all_composed.compatibility.errors.iter().any(|error| {
                error.contains("Codex cross-tier") && error.contains("fork_turns all")
            });
    if !all_fork_rejected {
        errors.push("Sol/Terra/Luna fork_turns=all was not rejected".to_string());
    }
    let metrics = ProjectedMetrics {
        tool_calls: 1,
        tokens: 1,
        credits_micros: 1,
        latency_ms: 1,
    };
    let missing = route_observation_for(
        &policy,
        &binding,
        "codex-terra-high",
        "codex",
        Some("planr-terra-high"),
        "gpt-5.6-terra",
        Some("high"),
        ContextForkMode::None,
        false,
        FixtureTransition::AvailabilityFallback,
        "sol-terra-luna-route-proof",
        &metrics,
    );
    let missing_effective_evidence_cannot_verify =
        validate_route_observation(&missing).is_ok() && !route_is_verified(&missing);
    if !missing_effective_evidence_cannot_verify {
        errors
            .push("missing effective model/effort evidence passed route verification".to_string());
    }
    let verified = route_observation_for(
        &policy,
        &binding,
        "codex-terra-high",
        "codex",
        Some("planr-terra-high"),
        "gpt-5.6-terra",
        Some("high"),
        ContextForkMode::None,
        true,
        FixtureTransition::AvailabilityFallback,
        "sol-terra-luna-route-proof",
        &metrics,
    );
    let verified_effective_evidence_passes =
        validate_route_observation(&verified).is_ok() && route_is_verified(&verified);
    if !verified_effective_evidence_passes {
        errors.push("locally evidenced Sol/Terra/Luna route did not verify".to_string());
    }
    let pass = errors.is_empty();
    Ok(CodexDispatchContract {
        fixture: VersionReference {
            id: binding.id.clone(),
            version: binding.version.clone(),
        },
        model_versions: model_provenance(&binding),
        all_fork_rejected,
        none_fork_parameters_verified,
        missing_effective_evidence_cannot_verify,
        verified_effective_evidence_passes,
        errors,
        pass,
    })
}

fn model_provenance(binding: &HostBindingV1) -> BTreeMap<String, ModelProvenance> {
    binding
        .profiles
        .iter()
        .map(|(role, profile)| {
            (
                role.clone(),
                ModelProvenance {
                    client: profile.client.clone(),
                    model: profile.model.clone(),
                    effort: profile.effort.clone(),
                },
            )
        })
        .collect()
}

fn transition_name(transition: FixtureTransition) -> &'static str {
    match transition {
        FixtureTransition::Retry => "retry",
        FixtureTransition::AvailabilityFallback => "availability_fallback",
        FixtureTransition::QualityEscalation => "quality_escalation",
        FixtureTransition::QuotaDowngrade => "quota_downgrade",
        FixtureTransition::SafetyStop => "safety_stop",
    }
}

fn route_transition(transition: FixtureTransition) -> RouteTransitionKind {
    match transition {
        FixtureTransition::Retry => RouteTransitionKind::Retry,
        FixtureTransition::AvailabilityFallback => RouteTransitionKind::AvailabilityFallback,
        FixtureTransition::QualityEscalation => RouteTransitionKind::QualityEscalation,
        FixtureTransition::QuotaDowngrade => RouteTransitionKind::QuotaDowngrade,
        FixtureTransition::SafetyStop => RouteTransitionKind::SafetyStop,
    }
}

fn hash_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| format!("evaluation evidence serialization failed: {error}"))
}

pub(crate) fn render_markdown(report: &PresetEvaluationReport) -> String {
    let mut markdown = format!(
        "# Preset Evaluation Verification\n\nFixture: `{}` v{} (`{}`)  \nVerified: {} / expires: {} / evaluated: {}  \nRunner: {} schema {} / Planr {} / {} {}\n\n",
        report.suite.id,
        report.suite.version,
        report.suite.fixture_sha256,
        report.suite.verified_at_unix,
        report.suite.expires_at_unix,
        report.suite.evaluated_at_unix,
        report.environment.runner,
        report.environment.runner_schema_version,
        report.environment.planr_version,
        report.environment.operating_system,
        report.environment.architecture,
    );
    if report.environment.runner == "planr-offline-policy-simulator" {
        markdown.push_str("This command is an offline policy simulation: it does not execute task workflows or observe effective host routes, so it cannot produce recommendations.\n\n");
    } else if report.environment.runner == "planr-instrumented-live-host-runner" {
        markdown.push_str("This report joined Planr-read challenge-bound task artifacts with independently signed Ed25519 telemetry receipts. Verified receipts provide trusted effective-route and usage measurements; missing or invalid receipts remain recommendation-ineligible.\n\n");
    } else {
        markdown.push_str("This report was produced by the explicit live-host runner; Planr-read challenge-bound task artifacts, candidate/task oracles, estimated host route/usage claims, and observed process latency are included in each result. Arbitrary-process claims remain recommendation-ineligible without independent instrumentation.\n\n");
    }
    markdown.push_str("| Candidate | Status | Source | Quality | Reliability | Avg cost | p95 latency | Hashes | Label |\n");
    markdown.push_str("|---|---|---|---:|---:|---:|---:|---:|---|\n");
    for candidate in &report.candidates {
        markdown.push_str(&format!(
            "| `{}` | `{:?}` | `{:?}` | {} bps | {} bps | {} μcredits | {} ms | {}/{} | {} |\n",
            candidate.id,
            candidate.status,
            candidate.metrics.metrics_source,
            candidate.metrics.average_quality_score_bps,
            candidate.metrics.reliability_bps,
            candidate.metrics.average_credits_micros,
            candidate.metrics.p95_latency_ms,
            candidate.metrics.verified_result_hashes,
            candidate.metrics.runs,
            candidate.label.as_deref().unwrap_or("—"),
        ));
    }
    markdown.push_str("\n## Versioned task inputs\n\n");
    for task in &report.task_fixtures {
        markdown.push_str(&format!(
            "- `{:?}` `{}` v{}: `{}`\n",
            task.kind, task.id, task.version, task.input_sha256
        ));
    }
    markdown.push_str("\n## Codex Sol/Terra/Luna contract\n\n");
    markdown.push_str(&format!(
        "- [{}] `fork_turns = all` rejected\n- [{}] `fork_turns = none` parameters verified\n- [{}] missing effective model/effort cannot verify\n- [{}] process-exit effective route evidence verifies\n",
        check(report.codex_dispatch_contract.all_fork_rejected),
        check(report.codex_dispatch_contract.none_fork_parameters_verified),
        check(report.codex_dispatch_contract.missing_effective_evidence_cannot_verify),
        check(report.codex_dispatch_contract.verified_effective_evidence_passes),
    ));
    markdown.push_str(&format!(
        "\nReproducible evidence: **{}**.\n",
        if report.reproducible_evidence {
            "pass"
        } else {
            "fail"
        }
    ));
    markdown
}

fn check(pass: bool) -> &'static str {
    if pass { "x" } else { " " }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeLiveHost {
        request_bound: bool,
    }

    impl LiveHostAdapter for FakeLiveHost {
        fn run(&self, request: &LiveHostRequest) -> Result<LiveHostResult, String> {
            let suffix = match request.task.id.as_str() {
                "explore-routing-boundaries" => "routing-boundaries-inspected",
                "implement-bounded-policy-change" => "bounded-policy-change-implemented",
                "mechanical-schema-rewrite" => "owned-schema-rewritten",
                "browser-report-smoke" => "browser-report-inspected",
                "visual-report-regression" => "visual-contract-matched",
                "security-safety-stop" => "unsafe-operation-stopped",
                "subagent-sol-terra-luna-dispatch" => "sol-terra-luna-dispatch-verified",
                _ => "unknown-task",
            };
            let (candidate_id, task_id, input_sha256, artifact_kind, output) = if self.request_bound
            {
                (
                    request.candidate.id.clone(),
                    request.task.id.clone(),
                    request.input_sha256.clone(),
                    request.artifact_kind.clone(),
                    format!("{}:{}:{suffix}", request.candidate.id, request.task.id),
                )
            } else {
                (
                    "constant-candidate".to_string(),
                    "constant-task".to_string(),
                    "constant-input".to_string(),
                    "constant-artifact".to_string(),
                    "pass".to_string(),
                )
            };
            let artifact_sha256 = if self.request_bound {
                let challenge_bytes = fs::read(&request.challenge_path).unwrap();
                let artifact = LiveTaskArtifact {
                    schema_version: 1,
                    candidate_id: candidate_id.clone(),
                    task_id: task_id.clone(),
                    input_sha256: input_sha256.clone(),
                    artifact_kind: artifact_kind.clone(),
                    challenge_sha256: sha256(&challenge_bytes),
                    output: output.clone(),
                };
                let artifact_bytes = serde_json::to_vec(&artifact).unwrap();
                fs::write(&request.artifact_path, &artifact_bytes).unwrap();
                sha256(&artifact_bytes)
            } else {
                String::new()
            };
            Ok(LiveHostResult {
                response: LiveHostResponse {
                    schema_version: 1,
                    host_id: "fake-host".to_string(),
                    host_version: "1.0.0".to_string(),
                    candidate_id,
                    task_id,
                    input_sha256,
                    artifact_kind,
                    artifact_sha256,
                    output,
                    effective_model: "gpt-5.6-terra".to_string(),
                    effective_effort: Some("high".to_string()),
                    effective_context_fork: ContextForkMode::None,
                    effective_agent_type: "planr-terra-high".to_string(),
                    effective_role: Some("planr-terra-high".to_string()),
                    thread_id: Some("thread-terra-high".to_string()),
                    status: Some("completed".to_string()),
                    tool_calls: 1,
                    tokens: 10,
                    credits_micros: 100,
                    retries: 1,
                    availability_fallbacks: 1,
                    quality_escalations: 1,
                    corrections: 1,
                    violations: 0,
                },
                latency_ms: 100,
            })
        }
    }

    fn options() -> EvaluationOptions {
        EvaluationOptions {
            at_unix: Some(1_783_987_200),
            host: None,
            live_host: None,
            trusted_telemetry: None,
        }
    }

    #[test]
    fn task_fixture_policy_simulations_are_hashed_and_never_recommended() {
        let report = evaluate_embedded_suite(&options()).unwrap();
        assert_eq!(report.task_fixtures.len(), 7);
        assert_eq!(report.candidates.len(), 4);
        for candidate in &report.candidates {
            assert_eq!(candidate.results.len(), 7);
            assert!(
                candidate
                    .results
                    .iter()
                    .all(|result| result.result_sha256.len() == 64)
            );
            assert!(candidate.results.iter().all(result_hash_valid));
            assert!(
                candidate
                    .results
                    .iter()
                    .all(|result| result.evidence.generated_by == "planr-offline-policy-simulator")
            );
            assert!(candidate.results.iter().all(|result| {
                !result.evidence.task_executed
                    && !result.evidence.outcome_oracle_evaluated
                    && !result.evidence.recommendation_eligible
                    && !result.evidence.route_verified
            }));
            assert_eq!(candidate.metrics.verified_route_runs, 0);
            assert_eq!(candidate.status, EvaluationStatus::Verified);
            assert!(!candidate.recommendation_evidence_complete);
        }
        assert!(report.recommended.is_empty());
    }

    #[test]
    fn live_process_artifacts_execute_oracles_but_cannot_self_attest_recommendations() {
        let live = evaluate_embedded_suite_with_adapter(
            &options(),
            || u64::MAX,
            Some(&FakeLiveHost {
                request_bound: true,
            }),
        )
        .unwrap();
        assert!(live.recommended.is_empty());
        for candidate in live.candidates.iter().filter(|candidate| {
            candidate.policy.id != "read-only-audit" && candidate.policy.id != "low-usage"
        }) {
            assert_eq!(candidate.evidence_scope, EvidenceScope::LiveTaskRun);
            assert_eq!(candidate.status, EvaluationStatus::Verified);
            assert!(candidate.evidence_complete);
            assert!(!candidate.recommendation_evidence_complete);
            assert!(candidate.status_reasons.iter().any(|reason| {
                reason.contains("live task artifacts were evaluated")
                    && reason.contains("no independently trusted route or cost instrumentation")
            }));
            assert_eq!(candidate.metrics.actual_task_runs, candidate.metrics.runs);
            assert_eq!(candidate.metrics.oracle_passes, candidate.metrics.runs);
            assert_eq!(candidate.metrics.verified_route_runs, 0);
            assert!(candidate.results.iter().all(|result| {
                result.evidence.task_executed
                    && result.evidence.outcome_oracle_evaluated
                    && !result.evidence.recommendation_eligible
                    && !result.evidence.route_verified
                    && result.evidence.metrics_source == MetricsSource::HostReported
                    && result.evidence.metering_confidence == MeteringMode::Estimated
                    && result
                        .evidence
                        .checks
                        .iter()
                        .any(|check| check.name == "challenge_binding" && check.pass)
                    && result
                        .evidence
                        .checks
                        .iter()
                        .any(|check| check.name == "trusted_metering" && !check.pass)
            }));
        }
        let read_only = live
            .candidates
            .iter()
            .find(|candidate| candidate.policy.id == "read-only-audit")
            .unwrap();
        assert_ne!(read_only.status, EvaluationStatus::Recommended);
        assert!(!read_only.recommendation_evidence_complete);
        assert!(read_only.results.iter().any(|result| {
            result.task_kind == TaskKind::Implementation
                && result
                    .evidence
                    .checks
                    .iter()
                    .any(|check| check.name == "write_capability" && !check.pass)
        }));

        let constant_response = evaluate_embedded_suite_with_adapter(
            &options(),
            || u64::MAX,
            Some(&FakeLiveHost {
                request_bound: false,
            }),
        )
        .unwrap();
        assert!(constant_response.recommended.is_empty());
        assert!(constant_response.candidates.iter().all(|candidate| {
            candidate.status == EvaluationStatus::Unverified
                && !candidate.evidence_complete
                && !candidate.recommendation_evidence_complete
                && candidate.status_reasons.iter().any(|reason| {
                    reason.contains("live execution was attempted")
                        && reason.contains("challenge-bound")
                })
        }));
        assert!(!constant_response.reproducible_evidence);
    }

    #[test]
    fn status_lifecycle_covers_verified_recommended_stale_and_incompatible() {
        let current = evaluate_embedded_suite(&options()).unwrap();
        assert!(
            current
                .candidates
                .iter()
                .all(|candidate| candidate.status == EvaluationStatus::Verified)
        );
        assert!(current.recommended.is_empty());

        let (recommended, _) = evaluation_status(true, true, true, true, true, 10, 20);
        assert_eq!(recommended, EvaluationStatus::Recommended);

        let stale = evaluate_embedded_suite(&EvaluationOptions {
            at_unix: Some(current.suite.expires_at_unix + 1),
            host: None,
            live_host: None,
            trusted_telemetry: None,
        })
        .unwrap();
        assert!(
            stale
                .candidates
                .iter()
                .all(|candidate| candidate.status == EvaluationStatus::Stale)
        );
        assert!(stale.recommended.is_empty());
        assert!(
            stale
                .candidates
                .iter()
                .all(|candidate| candidate.label.is_none())
        );

        let incompatible = evaluate_embedded_suite(&EvaluationOptions {
            at_unix: Some(1_783_987_200),
            host: Some("cursor".to_string()),
            live_host: None,
            trusted_telemetry: None,
        })
        .unwrap();
        assert!(
            incompatible
                .candidates
                .iter()
                .all(|candidate| candidate.status == EvaluationStatus::Incompatible)
        );
        assert!(incompatible.recommended.is_empty());
    }

    #[test]
    fn default_evaluation_time_uses_the_clock_and_override_remains_deterministic() {
        let stale =
            evaluate_embedded_suite_with_clock(&EvaluationOptions::default(), || 1_815_523_201)
                .unwrap();
        assert_eq!(stale.suite.evaluated_at_unix, 1_815_523_201);
        assert!(
            stale
                .candidates
                .iter()
                .all(|candidate| candidate.status == EvaluationStatus::Stale)
        );

        let override_time = evaluate_embedded_suite_with_clock(&options(), || u64::MAX).unwrap();
        assert_eq!(override_time.suite.evaluated_at_unix, 1_783_987_200);
        assert!(
            override_time
                .candidates
                .iter()
                .all(|candidate| candidate.status == EvaluationStatus::Verified)
        );
    }

    #[test]
    fn binding_only_offline_simulation_cannot_claim_effective_route_evidence() {
        let report = evaluate_embedded_suite(&options()).unwrap();
        for result in report
            .candidates
            .iter()
            .flat_map(|candidate| &candidate.results)
        {
            let effective = &result.evidence.route_observation.effective;
            assert_eq!(effective.model.enforcement, EnforcementState::Unavailable);
            assert_eq!(effective.effort.enforcement, EnforcementState::Unavailable);
            assert_eq!(
                effective.context_fork.enforcement,
                EnforcementState::Unavailable
            );
            assert!(!result.evidence.route_verified);
        }
    }

    #[test]
    fn sol_terra_luna_contract_fails_all_and_requires_effective_evidence() {
        let report = evaluate_sol_terra_luna_contract().unwrap();
        assert!(report.pass, "{:?}", report.errors);
        assert!(report.all_fork_rejected);
        assert!(report.none_fork_parameters_verified);
        assert!(report.missing_effective_evidence_cannot_verify);
        assert!(report.verified_effective_evidence_passes);
    }

    #[test]
    fn reports_include_fixture_environment_model_and_hash_provenance() {
        let report = evaluate_embedded_suite(&options()).unwrap();
        let machine = serde_json::to_value(&report).unwrap();
        assert_eq!(machine["schema_version"], 1);
        assert_eq!(machine["suite"]["version"], "1.8.0");
        assert_eq!(
            machine["environment"]["runner"],
            "planr-offline-policy-simulator"
        );
        assert_eq!(
            machine["suite"]["fixture_sha256"].as_str().unwrap().len(),
            64
        );
        assert_eq!(
            machine["candidates"][0]["model_versions"]["worker"]["model"],
            "gpt-5.6-terra"
        );
        assert_eq!(
            machine["candidates"][0]["threshold_results"]
                .as_array()
                .unwrap()
                .len(),
            11
        );
        assert_eq!(
            machine["candidates"][0]["results"][0]["result_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        let markdown = render_markdown(&report);
        assert!(markdown.contains("# Preset Evaluation Verification"));
        assert!(markdown.contains("offline policy simulation"));
        assert!(markdown.contains("Versioned task inputs"));
    }

    #[test]
    fn route_verified_requires_exact_effective_agent_type() {
        let policy = parse_policy(builtin_policy(Path::new("balanced")).unwrap().content).unwrap();
        let binding =
            parse_host_binding(builtin_binding(Path::new("codex-openai")).unwrap().content)
                .unwrap();
        let metrics = ProjectedMetrics {
            tool_calls: 1,
            tokens: 1,
            credits_micros: 1,
            latency_ms: 1,
        };
        let route = route_observation_for(
            &policy,
            &binding,
            "codex-terra-high",
            "codex",
            Some("planr-terra-high"),
            "gpt-5.6-terra",
            Some("high"),
            ContextForkMode::None,
            true,
            FixtureTransition::AvailabilityFallback,
            "agent-type-proof",
            &metrics,
        );
        assert!(route_is_verified(&route));

        let mut missing = route.clone();
        missing.effective.agent_type = unavailable_string();
        assert!(!route_is_verified(&missing));

        let mut mismatched = route;
        mismatched.effective.agent_type.value = Some("planr-sol-high".to_string());
        assert!(!route_is_verified(&mismatched));
    }
}
