#![allow(dead_code)]

use crate::canonical_json::{sha256_json_digest, sha256_prefixed_bytes};
use crate::evidence::model::{
    CapabilityAvailabilityStatus, EvidenceId, VerificationCapabilityInstance,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(crate) const HOST_CAPABILITY_RAW_SCHEMA_V1: &str = "planr.host_capability_observed_raw.v1";
pub(crate) const HOST_CAPABILITY_RAW_PAYLOAD_V1: &str = "host-capability-raw/1.0.0";

#[derive(Debug, Clone)]
pub(crate) struct HostCaptureEvaluation {
    pub experiment_id: String,
    pub host: String,
    pub surface: String,
    pub input_kind: String,
    pub tool_name: String,
    pub event_source: String,
    pub final_status: CapabilityAvailabilityStatus,
    pub claim_source: String,
    pub source_kind: String,
    pub observation_mode: String,
    pub external_producer: Option<Value>,
    pub availability_reason: String,
    pub missing_fields: Vec<String>,
    pub artifact_refs: Vec<HostArtifactRef>,
    pub raw_digest: String,
    pub raw_schema_digest: String,
    pub provenance_digest: String,
    pub instance: VerificationCapabilityInstance,
    pub instance_value: Value,
    pub final_event_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostArtifactRef {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentSuite {
    schema_version: String,
    suite_id: String,
    docs_are_experiment_design_only: bool,
    #[serde(default)]
    source_docs: Vec<Value>,
    experiments: Vec<ExperimentDesign>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentDesign {
    id: String,
    host: String,
    surface: String,
    input_kind: String,
    expected_tool_name: String,
    expected_event_source: String,
    expected_result_status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapture {
    schema_version: String,
    payload_version: String,
    experiment_id: String,
    host_identity: HostIdentity,
    surface: String,
    tool_name: String,
    event_source: String,
    started_at: String,
    ended_at: String,
    input: RawInput,
    events: Vec<RawEvent>,
    result: RawResult,
    provenance_ref: FileRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostIdentity {
    host: String,
    surface: String,
    version: String,
    adapter_version: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawInput {
    input_kind: String,
    #[serde(flatten)]
    _extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvent {
    sequence: u64,
    event_name: String,
    #[serde(default, rename = "final")]
    final_: Option<bool>,
    payload_version: String,
    tool_name: String,
    event_source: String,
    payload: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResult {
    final_status: String,
    permissions: Value,
    sandbox: Value,
    missing_fields: Vec<String>,
    blind_spots: Vec<String>,
    artifact_refs: Vec<RawArtifactRef>,
    artifact_digests: BTreeMap<String, String>,
    availability_reason: String,
    #[serde(default)]
    experiment_plan: Vec<String>,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArtifactRef {
    id: String,
    kind: String,
    root_kind: String,
    path: String,
    digest: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRef {
    path: String,
    digest: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedManifest {
    schema_version: String,
    payload_version: String,
    suite_id: String,
    schema_refs: ExpectedSchemaRefs,
    provenance_ref: FileRef,
    capability_instances: Vec<ExpectedCapabilityEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedSchemaRefs {
    raw: FileRef,
    expected: FileRef,
    provenance: FileRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedCapabilityEntry {
    raw_capture_id: String,
    claim_source: String,
    trusted_adapter_enabled: bool,
    capability_instance: Value,
    manifest_ref: FileRef,
    provenance_ref: FileRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceFile {
    schema_version: String,
    schema_ref: String,
    schema_digest: String,
    suite_id: String,
    captures: Vec<ProvenanceCapture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvenanceCapture {
    experiment_id: String,
    source_kind: String,
    host: String,
    surface: String,
    input_kind: String,
    tool_name: String,
    event_source: String,
    host_version: String,
    adapter_version: String,
    claim_source: String,
    availability_reason: String,
    probe_checks: Vec<Value>,
    missing_fields: Vec<String>,
    artifact_ids: Vec<String>,
    captured_at: String,
    observation_mode: String,
    #[serde(default)]
    external_producer: Option<Value>,
}

#[derive(Debug, Clone)]
struct StrictPhase1Contract {
    raw_documents: BTreeMap<String, Value>,
    raw_digests: BTreeMap<String, String>,
    expected_document: Value,
    provenance_document: Value,
    raw_schema_digest: String,
    expected_schema_digest: String,
    provenance_schema_digest: String,
    provenance_digest: String,
}

pub(crate) fn evaluate_phase1_host_fixture(
    fixture_root: &Path,
) -> Result<Vec<HostCaptureEvaluation>> {
    let suite: ExperimentSuite = read_json(fixture_root, "experiment-suite.json")?;
    if suite.schema_version != "planr.host_capability_experiment_suite.v1"
        || !suite.docs_are_experiment_design_only
        || suite.suite_id.is_empty()
    {
        bail!("host capability suite header drifted");
    }
    let _source_docs_are_design_only = suite.source_docs.len();

    let strict = validate_strict_phase1_contract(fixture_root)?;
    let expected: ExpectedManifest = serde_json::from_value(strict.expected_document.clone())
        .context("parsing expected/normalized-manifest.json")?;
    if expected.schema_version != "planr.host_capability_expected_manifest.v1"
        || expected.payload_version != "host-capability-expected/1.0.0"
        || expected.suite_id != suite.suite_id
    {
        bail!("host capability expected manifest header drifted");
    }

    let provenance_rel = "provenance/host-capability-captures.json";
    let manifest_rel = "manifests/phase1-host-capability-manifests.json";
    let manifest_digest = sha256_prefixed_bytes(
        &fs::read(fixture_root.join(manifest_rel))
            .with_context(|| format!("reading {manifest_rel}"))?,
    );
    let raw_schema_digest = strict.raw_schema_digest.clone();
    let provenance_digest = strict.provenance_digest.clone();
    let provenance: ProvenanceFile = serde_json::from_value(strict.provenance_document.clone())
        .with_context(|| format!("parsing {provenance_rel}"))?;
    if provenance.schema_version != "planr.host_capability_provenance.v1"
        || provenance.schema_ref != "schemas/host-capability-provenance.schema.json"
        || provenance.schema_digest != strict.provenance_schema_digest
        || provenance.suite_id != suite.suite_id
    {
        bail!("host capability provenance header drifted");
    }
    validate_expected_schema_refs(&expected, &strict)?;

    let expected_by_id = expected
        .capability_instances
        .iter()
        .map(|entry| (entry.raw_capture_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let provenance_by_id = provenance
        .captures
        .iter()
        .map(|capture| (capture.experiment_id.as_str(), capture))
        .collect::<BTreeMap<_, _>>();
    let mut artifact_ids = BTreeSet::new();
    let mut evaluations = Vec::new();

    for experiment in &suite.experiments {
        let raw_rel = format!("observed/{}.json", experiment.id);
        let raw_value = strict
            .raw_documents
            .get(experiment.id.as_str())
            .with_context(|| format!("missing strict raw document {}", experiment.id))?;
        let raw_digest = strict
            .raw_digests
            .get(experiment.id.as_str())
            .with_context(|| format!("missing strict raw digest {}", experiment.id))?
            .clone();
        let raw: RawCapture = serde_json::from_value(raw_value.clone())
            .with_context(|| format!("parsing {raw_rel}"))?;
        validate_payload_version(&raw.schema_version, &raw.payload_version)?;
        validate_raw_capture(&raw, experiment)?;
        let final_event_payload = final_event(&raw)?.payload.clone();
        let expected_entry = expected_by_id
            .get(experiment.id.as_str())
            .with_context(|| format!("missing expected capability entry {}", experiment.id))?;
        let provenance_capture = provenance_by_id
            .get(experiment.id.as_str())
            .with_context(|| format!("missing provenance capture {}", experiment.id))?;
        validate_expected_entry(
            expected_entry,
            &raw,
            provenance_rel,
            &provenance_digest,
            manifest_rel,
            &manifest_digest,
        )?;
        validate_provenance_capture(fixture_root, provenance_capture, &raw)?;
        let artifacts = validate_artifacts(fixture_root, &raw, &mut artifact_ids)?;
        let instance: VerificationCapabilityInstance = serde_json::from_value(
            expected_entry.capability_instance.clone(),
        )
        .with_context(|| format!("canonical capability instance rejected {}", experiment.id))?;

        evaluations.push(HostCaptureEvaluation {
            experiment_id: experiment.id.clone(),
            host: raw.host_identity.host,
            surface: raw.surface,
            input_kind: raw.input.input_kind,
            tool_name: raw.tool_name,
            event_source: raw.event_source,
            final_status: CapabilityAvailabilityStatus::from_str(&raw.result.final_status)
                .map_err(|error| anyhow::anyhow!(error))?,
            claim_source: expected_entry.claim_source.clone(),
            source_kind: provenance_capture.source_kind.clone(),
            observation_mode: provenance_capture.observation_mode.clone(),
            external_producer: provenance_capture.external_producer.clone(),
            availability_reason: raw.result.availability_reason,
            missing_fields: raw.result.missing_fields,
            artifact_refs: artifacts,
            raw_digest,
            raw_schema_digest: raw_schema_digest.clone(),
            provenance_digest: provenance_digest.clone(),
            instance,
            instance_value: expected_entry.capability_instance.clone(),
            final_event_payload,
        });
    }

    Ok(evaluations)
}

fn validate_strict_phase1_contract(fixture_root: &Path) -> Result<StrictPhase1Contract> {
    let raw_schema = read_json_value(
        fixture_root,
        "schemas/host-capability-observed-raw.schema.json",
    )?;
    let expected_schema = read_json_value(
        fixture_root,
        "schemas/host-capability-expected-manifest.schema.json",
    )?;
    let provenance_schema = read_json_value(
        fixture_root,
        "schemas/host-capability-provenance.schema.json",
    )?;
    validate_contract_schema(&raw_schema, "schemas.raw")?;
    validate_contract_schema(&expected_schema, "schemas.expected")?;
    validate_contract_schema(&provenance_schema, "schemas.provenance")?;

    let raw_schema_digest = file_digest(
        fixture_root,
        "schemas/host-capability-observed-raw.schema.json",
    )?;
    let expected_schema_digest = file_digest(
        fixture_root,
        "schemas/host-capability-expected-manifest.schema.json",
    )?;
    let provenance_schema_digest = file_digest(
        fixture_root,
        "schemas/host-capability-provenance.schema.json",
    )?;
    let expected_document = read_json_value(fixture_root, "expected/normalized-manifest.json")?;
    let provenance_document =
        read_json_value(fixture_root, "provenance/host-capability-captures.json")?;
    let provenance_digest = file_digest(fixture_root, "provenance/host-capability-captures.json")?;

    validate_json_schema_instance(&expected_schema, &expected_document, "expected_document")?;
    validate_json_schema_instance(
        &provenance_schema,
        &provenance_document,
        "provenance_document",
    )?;

    let mut raw_documents = BTreeMap::new();
    let mut raw_digests = BTreeMap::new();
    for experiment in
        read_json::<ExperimentSuite>(fixture_root, "experiment-suite.json")?.experiments
    {
        let raw_rel = format!("observed/{}.json", experiment.id);
        let raw = read_json_value(fixture_root, &raw_rel)?;
        validate_json_schema_instance(&raw_schema, &raw, &raw_rel)?;
        raw_digests.insert(experiment.id.clone(), file_digest(fixture_root, &raw_rel)?);
        raw_documents.insert(experiment.id, raw);
    }

    Ok(StrictPhase1Contract {
        raw_documents,
        raw_digests,
        expected_document,
        provenance_document,
        raw_schema_digest,
        expected_schema_digest,
        provenance_schema_digest,
        provenance_digest,
    })
}

fn read_json_value(root: &Path, relative: &str) -> Result<Value> {
    read_json(root, relative)
}

fn file_digest(root: &Path, relative: &str) -> Result<String> {
    let bytes = fs::read(root.join(relative)).with_context(|| format!("reading {relative}"))?;
    Ok(sha256_prefixed_bytes(&bytes))
}

fn validate_contract_schema(schema: &Value, label: &str) -> Result<()> {
    let object = schema
        .as_object()
        .with_context(|| format!("{label} must be a JSON Schema object"))?;
    if object
        .get("properties")
        .and_then(Value::as_object)
        .is_none_or(|properties| properties.is_empty())
    {
        bail!("{label} must not be a permissive empty schema");
    }
    if object.get("$schema").and_then(Value::as_str).is_none() {
        bail!("{label} must declare $schema");
    }
    if object.get("additionalProperties") != Some(&Value::Bool(false)) {
        bail!("{label} root object must set additionalProperties false");
    }
    if object
        .get("required")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        bail!("{label} root object must declare required fields");
    }
    jsonschema::draft202012::options()
        .build(schema)
        .with_context(|| format!("compiling {label}"))?;
    Ok(())
}

fn validate_json_schema_instance(schema: &Value, instance: &Value, label: &str) -> Result<()> {
    let validator = jsonschema::draft202012::options()
        .build(schema)
        .with_context(|| format!("compiling schema for {label}"))?;
    if let Err(error) = validator.validate(instance) {
        bail!("{label} failed schema validation: {error}");
    }
    Ok(())
}

fn validate_expected_schema_refs(
    expected: &ExpectedManifest,
    strict: &StrictPhase1Contract,
) -> Result<()> {
    if expected.schema_refs.raw.path != "schemas/host-capability-observed-raw.schema.json"
        || expected.schema_refs.raw.digest != strict.raw_schema_digest
        || expected.schema_refs.expected.path
            != "schemas/host-capability-expected-manifest.schema.json"
        || expected.schema_refs.expected.digest != strict.expected_schema_digest
        || expected.schema_refs.provenance.path != "schemas/host-capability-provenance.schema.json"
        || expected.schema_refs.provenance.digest != strict.provenance_schema_digest
        || expected.provenance_ref.path != "provenance/host-capability-captures.json"
        || expected.provenance_ref.digest != strict.provenance_digest
    {
        bail!("host capability expected schema/provenance refs drifted");
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(root: &Path, relative: &str) -> Result<T> {
    let path = root.join(relative);
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn validate_payload_version(schema_version: &str, payload_version: &str) -> Result<()> {
    if schema_version != HOST_CAPABILITY_RAW_SCHEMA_V1 {
        let major = schema_version
            .rsplit_once(".v")
            .and_then(|(_, major)| major.parse::<u64>().ok())
            .unwrap_or(0);
        bail!("unsupported major version {major}");
    }
    if payload_version != HOST_CAPABILITY_RAW_PAYLOAD_V1 {
        let Some(version) = payload_version.strip_prefix("host-capability-raw/") else {
            bail!("unsupported host capability payload contract {payload_version}");
        };
        let parts = version.split('.').collect::<Vec<_>>();
        let major = parts.first().copied().unwrap_or("0");
        let minor = parts.get(1).copied().unwrap_or("0");
        if major != "1" {
            bail!("unsupported major version {major}");
        }
        bail!("unsupported minor version {minor}");
    }
    Ok(())
}

fn validate_raw_capture(raw: &RawCapture, experiment: &ExperimentDesign) -> Result<()> {
    if raw.experiment_id != experiment.id
        || raw.host_identity.host != experiment.host
        || raw.host_identity.surface != experiment.surface
        || raw.surface != experiment.surface
        || raw.input.input_kind != experiment.input_kind
        || raw.tool_name != experiment.expected_tool_name
        || raw.event_source != experiment.expected_event_source
        || raw.result.final_status != experiment.expected_result_status
    {
        bail!(
            "host capability raw capture does not match experiment {}",
            experiment.id
        );
    }
    if raw.host_identity.version.is_empty()
        || raw.host_identity.adapter_version != "planr-host-experiment-harness/1.0.0"
        || raw.started_at.is_empty()
        || raw.ended_at.is_empty()
        || raw.result.blind_spots.is_empty()
    {
        bail!("host capability raw capture missing bound host/result fields");
    }
    let started_at = OffsetDateTime::parse(&raw.started_at, &Rfc3339)
        .context("host capability started_at is not RFC3339")?;
    let ended_at = OffsetDateTime::parse(&raw.ended_at, &Rfc3339)
        .context("host capability ended_at is not RFC3339")?;
    if ended_at < started_at {
        bail!("host capability ended_at is before started_at");
    }
    let _permission_and_sandbox_are_present = (&raw.result.permissions, &raw.result.sandbox);
    let _notes_are_contract_checked = raw.result.notes.len();
    Ok(())
}

fn final_event(raw: &RawCapture) -> Result<&RawEvent> {
    let mut previous_sequence = 0;
    let mut seen = BTreeSet::new();
    for event in &raw.events {
        if event.sequence <= previous_sequence || !seen.insert(event.sequence) {
            bail!("host capability event sequences must be ordered and unique");
        }
        previous_sequence = event.sequence;
    }
    let finals = raw
        .events
        .iter()
        .filter(|event| event.final_ == Some(true))
        .collect::<Vec<_>>();
    let [event] = finals.as_slice() else {
        bail!("host capability capture must contain exactly one final event");
    };
    if raw.events.last().map(|last| last.sequence) != Some(event.sequence) {
        bail!("host capability final event must be the last event");
    }
    if event.event_name != "final"
        || event.payload_version != "host-event/1.0.0"
        || event.tool_name != raw.tool_name
        || event.event_source != raw.event_source
    {
        bail!("host capability final event is not bound to raw result");
    }
    let final_status = event
        .payload
        .get("final_status")
        .and_then(Value::as_str)
        .context("host capability final event payload.final_status is required")?;
    if final_status != raw.result.final_status {
        bail!("host capability final event status is not bound to raw result");
    }
    Ok(event)
}

fn validate_expected_entry(
    entry: &ExpectedCapabilityEntry,
    raw: &RawCapture,
    provenance_rel: &str,
    provenance_digest: &str,
    manifest_rel: &str,
    manifest_digest: &str,
) -> Result<()> {
    if entry.trusted_adapter_enabled {
        bail!("phase1 fixture must not pre-enable trusted adapters");
    }
    if entry.raw_capture_id != raw.experiment_id
        || entry.provenance_ref.path != provenance_rel
        || entry.provenance_ref.digest != provenance_digest
        || raw.provenance_ref.path != provenance_rel
        || raw.provenance_ref.digest != provenance_digest
    {
        bail!("host capability expected entry is not bound to provenance");
    }
    let status = entry
        .capability_instance
        .pointer("/availability/status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status != raw.result.final_status {
        bail!("host capability expected availability does not match raw status");
    }
    let reason = entry
        .capability_instance
        .pointer("/availability/reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason != raw.result.availability_reason {
        bail!("host capability expected availability reason does not match raw");
    }
    if entry.manifest_ref.path != manifest_rel || entry.manifest_ref.digest != manifest_digest {
        bail!("host capability expected manifest ref drifted");
    }
    Ok(())
}

fn validate_provenance_capture(
    fixture_root: &Path,
    capture: &ProvenanceCapture,
    raw: &RawCapture,
) -> Result<()> {
    if capture.host != raw.host_identity.host
        || capture.surface != raw.surface
        || capture.input_kind != raw.input.input_kind
        || capture.tool_name != raw.tool_name
        || capture.event_source != raw.event_source
        || capture.host_version != raw.host_identity.version
        || capture.adapter_version != raw.host_identity.adapter_version
        || capture.claim_source != expected_claim_source(raw)
        || capture.availability_reason != raw.result.availability_reason
        || capture.missing_fields != raw.result.missing_fields
        || capture.captured_at != raw.ended_at
    {
        bail!("host capability provenance capture drifted from raw capture");
    }
    validate_claim_source_binding(fixture_root, capture, raw)?;
    let _probe_checks_are_bound = capture.probe_checks.len();
    let _external_producer_is_preserved = capture.external_producer.as_ref();
    let mut raw_artifact_ids = raw
        .result
        .artifact_refs
        .iter()
        .map(|artifact| artifact.id.as_str())
        .collect::<Vec<_>>();
    let mut provenance_artifact_ids = capture
        .artifact_ids
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    raw_artifact_ids.sort_unstable();
    provenance_artifact_ids.sort_unstable();
    if raw_artifact_ids != provenance_artifact_ids {
        bail!("host capability provenance artifact ids drifted from raw capture");
    }
    Ok(())
}

fn expected_claim_source(raw: &RawCapture) -> &'static str {
    match raw.input.input_kind.as_str() {
        "unprobed_placeholder" => "capture_mode_placeholder",
        "mechanical_availability_probe" => "mechanical_unavailable_probe",
        _ => "observed_capture",
    }
}

fn validate_claim_source_binding(
    fixture_root: &Path,
    capture: &ProvenanceCapture,
    raw: &RawCapture,
) -> Result<()> {
    match capture.claim_source.as_str() {
        "observed_capture" => {
            if capture.source_kind != "external_observed_capture"
                || capture.observation_mode != "observed_payload"
                || raw.input.input_kind != "controlled_probe"
            {
                bail!(
                    "host capability observed_capture provenance is not valid for raw input_kind"
                );
            }
        }
        "mechanical_unavailable_probe" => {
            if capture.source_kind != "mechanical_unavailable_probe"
                || capture.observation_mode != "mechanical_invocation"
                || raw.input.input_kind != "mechanical_availability_probe"
            {
                bail!(
                    "host capability mechanical_unavailable_probe provenance is not mechanically bound"
                );
            }
            validate_mechanical_unavailable_probe(fixture_root, capture, raw)?;
        }
        "capture_mode_placeholder" => {
            if capture.source_kind != "unprobed_placeholder"
                || capture.observation_mode != "unprobed_placeholder"
                || raw.input.input_kind != "unprobed_placeholder"
            {
                bail!(
                    "host capability capture_mode_placeholder provenance is not placeholder-bound"
                );
            }
        }
        _ => bail!("host capability claim_source is unsupported"),
    }
    Ok(())
}

fn validate_mechanical_unavailable_probe(
    fixture_root: &Path,
    capture: &ProvenanceCapture,
    raw: &RawCapture,
) -> Result<()> {
    if raw.result.final_status == "available" {
        bail!("host capability mechanical_unavailable_probe must not claim available");
    }
    if capture.external_producer.is_some() {
        bail!(
            "host capability mechanical_unavailable_probe must not use external producer provenance"
        );
    }
    if raw.host_identity.host != "codex" {
        validate_peer_version_probe_artifacts(fixture_root, capture, raw)?;
        if !raw
            .result
            .missing_fields
            .iter()
            .any(|field| field == "final_result_payload")
        {
            bail!(
                "host capability mechanical_unavailable_probe must declare missing final_result_payload"
            );
        }
    }
    Ok(())
}

fn validate_peer_version_probe_artifacts(
    fixture_root: &Path,
    capture: &ProvenanceCapture,
    raw: &RawCapture,
) -> Result<()> {
    let command = raw
        .input
        ._extra
        .get("command")
        .and_then(Value::as_array)
        .context("host capability mechanical_unavailable_probe requires exact argv command")?;
    if command.is_empty() || !command.iter().all(|part| part.as_str().is_some()) {
        bail!("host capability mechanical_unavailable_probe command must be argv strings");
    }
    let artifact_by_kind = raw
        .result
        .artifact_refs
        .iter()
        .map(|artifact| (artifact.kind.as_str(), artifact))
        .collect::<BTreeMap<_, _>>();
    let stdout = artifact_by_kind
        .get("invocation-stdout")
        .context("host capability mechanical_unavailable_probe missing stdout artifact")?;
    let stderr = artifact_by_kind
        .get("invocation-stderr")
        .context("host capability mechanical_unavailable_probe missing stderr artifact")?;
    let result = artifact_by_kind
        .get("invocation-result")
        .context("host capability mechanical_unavailable_probe missing result artifact")?;
    let stdout_bytes = fs::read(contained_fixture_path(fixture_root, &stdout.path)?)
        .context("reading stdout artifact")?;
    let stderr_bytes = fs::read(contained_fixture_path(fixture_root, &stderr.path)?)
        .context("reading stderr artifact")?;
    let result_value: Value = serde_json::from_slice(
        &fs::read(contained_fixture_path(fixture_root, &result.path)?)
            .context("reading result artifact")?,
    )
    .context("parsing result artifact")?;
    if result_value.get("schema_version").and_then(Value::as_str)
        != Some("planr.host_capability_mechanical_invocation.v1")
    {
        bail!("host capability mechanical invocation schema_version is unsupported");
    }
    if result_value.get("argv") != Some(&Value::Array(command.clone())) {
        bail!("host capability mechanical invocation argv drifted from raw input");
    }
    if result_value
        .get("exit_status")
        .is_none_or(|status| status.as_i64().is_none())
    {
        bail!("host capability mechanical invocation exit_status is required");
    }
    if result_value.get("stdout_bytes").and_then(Value::as_u64) != Some(stdout_bytes.len() as u64) {
        bail!("host capability mechanical invocation stdout_bytes drifted");
    }
    if result_value.get("stderr_bytes").and_then(Value::as_u64) != Some(stderr_bytes.len() as u64) {
        bail!("host capability mechanical invocation stderr_bytes drifted");
    }
    let stdout_text = String::from_utf8(stdout_bytes).context("stdout artifact must be utf8")?;
    let version = parse_version_probe_stdout(&raw.host_identity.host, &stdout_text)
        .context("host capability mechanical_unavailable_probe could not parse host version")?;
    if raw.host_identity.version != version || capture.host_version != version {
        bail!("host capability host_version must be derived from invocation stdout artifact");
    }
    Ok(())
}

fn parse_version_probe_stdout(host: &str, stdout: &str) -> Option<String> {
    let trimmed = stdout.trim();
    match host {
        "claude" => trimmed
            .strip_suffix(" (Claude Code)")
            .filter(|version| version.split('.').count() == 3)
            .map(ToOwned::to_owned),
        "cursor" => {
            let (date, suffix) = trimmed.split_once('-')?;
            if date.split('.').count() == 3 && !suffix.is_empty() {
                Some(trimmed.to_owned())
            } else {
                None
            }
        }
        "pi" => {
            if trimmed.split('.').count() == 3 {
                Some(trimmed.to_owned())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn validate_artifacts(
    fixture_root: &Path,
    raw: &RawCapture,
    global_artifact_ids: &mut BTreeSet<String>,
) -> Result<Vec<HostArtifactRef>> {
    let mut local_digests = BTreeMap::new();
    let mut artifacts = Vec::new();
    for artifact in &raw.result.artifact_refs {
        if artifact.root_kind != "fixture_root" {
            bail!(
                "host capability artifact {} root_kind must be fixture_root",
                artifact.id
            );
        }
        EvidenceId::parse(artifact.id.clone()).map_err(|error| anyhow::anyhow!(error))?;
        if !global_artifact_ids.insert(artifact.id.clone()) {
            bail!("duplicate host capability artifact id {}", artifact.id);
        }
        let path = contained_fixture_path(fixture_root, &artifact.path)?;
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let digest = sha256_prefixed_bytes(&bytes);
        if digest != artifact.digest {
            bail!("host capability artifact {} digest mismatch", artifact.id);
        }
        local_digests.insert(artifact.id.clone(), digest);
        artifacts.push(HostArtifactRef {
            id: artifact.id.clone(),
            kind: artifact.kind.clone(),
            path: artifact.path.clone(),
            digest: artifact.digest.clone(),
        });
    }
    if local_digests != raw.result.artifact_digests {
        bail!("host capability artifact_digests do not match artifact_refs");
    }
    Ok(artifacts)
}

fn contained_fixture_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute() || relative.split('/').any(|part| part == "..") {
        bail!("host capability artifact path escapes fixture root: {relative}");
    }
    Ok(root.join(path))
}

pub(crate) fn capability_instance_digest(value: &Value) -> Result<String> {
    sha256_json_digest(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/evidence/host-capabilities/v1")
    }

    #[test]
    fn host_adapter_contract_accepts_phase1_fixture_and_rejects_version_drift() {
        let evaluations = evaluate_phase1_host_fixture(&fixture_root()).unwrap();
        assert_eq!(evaluations.len(), 14);
        assert_eq!(
            evaluations
                .iter()
                .filter(
                    |evaluation| evaluation.final_status == CapabilityAvailabilityStatus::Available
                )
                .count(),
            3
        );
        assert!(
            validate_payload_version(HOST_CAPABILITY_RAW_SCHEMA_V1, "host-capability-raw/2.0.0")
                .is_err()
        );
        assert!(
            validate_payload_version(HOST_CAPABILITY_RAW_SCHEMA_V1, "host-capability-raw/1.1.0")
                .is_err()
        );
    }

    #[test]
    fn host_adapter_contract_rejects_permissive_schema_and_unknown_fields() {
        let unknown_raw_result = copied_fixture();
        mutate_json(
            unknown_raw_result.path(),
            "observed/exp-chrome-browser-client.json",
            |value| {
                value["result"]["extra"] = json!(true);
            },
        );
        assert!(evaluate_phase1_host_fixture(unknown_raw_result.path()).is_err());

        let unknown_expected = copied_fixture();
        mutate_json(
            unknown_expected.path(),
            "expected/normalized-manifest.json",
            |value| {
                value["extra"] = json!(true);
            },
        );
        assert!(evaluate_phase1_host_fixture(unknown_expected.path()).is_err());

        let permissive_schema = copied_fixture();
        mutate_json(
            permissive_schema.path(),
            "schemas/host-capability-observed-raw.schema.json",
            |value| {
                *value = json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {}
                });
            },
        );
        assert!(evaluate_phase1_host_fixture(permissive_schema.path()).is_err());

        let provenance_digest_drift = copied_fixture();
        mutate_json(
            provenance_digest_drift.path(),
            "provenance/host-capability-captures.json",
            |value| {
                value["schema_digest"] = json!(
                    "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                );
            },
        );
        assert!(evaluate_phase1_host_fixture(provenance_digest_drift.path()).is_err());

        let missing_permissions = copied_fixture();
        mutate_json(
            missing_permissions.path(),
            "expected/normalized-manifest.json",
            |value| {
                let entry = value["capability_instances"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|entry| entry["raw_capture_id"] == "exp-claude-code-host-capture")
                    .unwrap();
                entry["capability_instance"]["permissions"]
                    .as_object_mut()
                    .unwrap()
                    .remove("network");
            },
        );
        assert!(evaluate_phase1_host_fixture(missing_permissions.path()).is_err());

        let host_upgrade = copied_fixture();
        mutate_json(
            host_upgrade.path(),
            "observed/exp-claude-code-host-capture.json",
            |value| {
                value["host_identity"]["version"] = json!("2.1.134");
            },
        );
        assert!(evaluate_phase1_host_fixture(host_upgrade.path()).is_err());
    }

    fn copied_fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        copy_dir(&fixture_root(), temp.path());
        temp
    }

    fn copy_dir(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn mutate_json(root: &Path, relative: &str, mutate: impl FnOnce(&mut Value)) {
        let path = root.join(relative);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        mutate(&mut value);
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}
