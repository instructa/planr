use super::model::{
    AdapterKind, CapabilityAvailabilityStatus, EvidenceReceipt, ProvenanceSourceKind,
    VerificationCapabilityInstance, VerificationCapabilityManifest,
};
use super::{
    AttemptStatus, EvidenceDomainError, EvidenceId, NamespacedIdentifier, SourceKind,
    UntrustedArtifactRef, UntrustedEvidenceProposal, reject_forbidden_authority_fields,
    reject_forbidden_authority_value,
};
use crate::canonical_json::{sha256_json_digest, sha256_prefixed_bytes};
use quick_xml::{Reader, events::Event};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Deserializer, de};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    fmt, fs,
    path::{Path, PathBuf},
    str::FromStr,
};

pub const VALIDATED_ARTIFACT_IMPORT_SCHEMA_VERSION: &str = "planr.evidence.import.v1";
pub const GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT: &str = "planr.generic_adapter_predicate.v1";
pub const PLANR_RUNNER_RESULT_FORMAT: &str = "planr.runner_result.v1";
pub const JUNIT_XML_FORMAT: &str = "junit.xml.v1";
const SUPPORTED_INNER_VERSION: &str = "1.0.0";
const RUNNER_OBSERVATION_TYPE: &str = "planr.runner.result";
const JUNIT_OBSERVATION_TYPE: &str = "junit.xml.suite";

#[derive(Debug)]
pub struct ValidatedArtifactImportRepository<'a> {
    pub conn: &'a Connection,
    pub project_id: &'a str,
    pub artifact_root: &'a Path,
}

#[derive(Debug, Clone)]
pub struct ValidatedImportRecord {
    pub id: String,
    pub digest: String,
    pub idempotent: bool,
    pub proposal: UntrustedEvidenceProposal,
}

#[derive(Debug, Clone)]
struct RegistryVerifierBinding {
    kind: String,
    id: String,
    name: String,
    version: String,
    digest: String,
    manifest_id: String,
    manifest_digest: String,
    instance_id: String,
    instance_digest: String,
    probe_execution_id: String,
    probe_result_digest: String,
}

impl RegistryVerifierBinding {
    fn to_value(&self) -> Value {
        json!({
            "kind": self.kind,
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "digest": self.digest,
            "manifest": {
                "id": self.manifest_id,
                "digest": self.manifest_digest,
            },
            "instance": {
                "id": self.instance_id,
                "digest": self.instance_digest,
                "probe_execution_id": self.probe_execution_id,
                "probe_result_digest": self.probe_result_digest,
            },
        })
    }
}

#[derive(Debug, Clone)]
struct ValidatedArtifactImport {
    id: String,
    submitted_at: String,
    artifact_refs: Vec<UntrustedArtifactRef>,
    verifier_identity: VerifierIdentity,
    evidence: ImportEvidence,
    producer_metadata: Map<String, Value>,
    format: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifierIdentity {
    kind: String,
    id: String,
    name: String,
    version: String,
    digest: String,
}

impl VerifierIdentity {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_import_field(&self.kind, "verifier_identity.kind")?;
        if self.kind == SourceKind::Agent.as_str() {
            return Err(EvidenceDomainError::ForbiddenTrustedField(
                "verifier_identity.kind",
            ));
        }
        EvidenceId::parse(self.id.clone())?;
        require_import_field(&self.name, "verifier_identity.name")?;
        require_import_field(&self.version, "verifier_identity.version")?;
        if self.version != SUPPORTED_INNER_VERSION {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "verifier_identity.version",
            ));
        }
        super::model::Sha256Digest::parse(self.digest.clone())?;
        Ok(())
    }

    fn to_value(&self) -> Value {
        json!({
            "kind": self.kind,
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "digest": self.digest,
        })
    }
}

#[derive(Debug, Clone)]
enum ImportEvidence {
    Generic(Box<GenericVersionedAdapterPredicate>),
    Runner(PlanrRunnerResult),
    Junit(JunitXmlEvidence),
}

impl ImportEvidence {
    fn observation_type(&self) -> &str {
        match self {
            Self::Generic(predicate) => &predicate.observation_type,
            Self::Runner(_) => RUNNER_OBSERVATION_TYPE,
            Self::Junit(_) => JUNIT_OBSERVATION_TYPE,
        }
    }

    fn claims(
        &self,
        artifacts: &[UntrustedArtifactRef],
        artifact_bytes: &[Vec<u8>],
        verifier: &RegistryVerifierBinding,
        repository: &ValidatedArtifactImportRepository<'_>,
    ) -> Result<Map<String, Value>, EvidenceDomainError> {
        let mut claims = Map::new();
        match self {
            Self::Generic(predicate) => {
                claims.insert(
                    "adapter_predicate".to_string(),
                    predicate.derived_claim(artifacts, artifact_bytes, verifier, repository)?,
                );
            }
            Self::Runner(result) => {
                claims.insert(
                    "runner_result".to_string(),
                    result.derived_claim(artifacts, artifact_bytes)?,
                );
            }
            Self::Junit(junit) => {
                claims.insert(
                    "junit".to_string(),
                    junit.derived_claim(artifacts, artifact_bytes)?,
                );
            }
        }
        Ok(claims)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericVersionedAdapterPredicate {
    kind: String,
    version: String,
    #[serde(rename = "type")]
    observation_type: String,
    outcome: String,
    predicate: Map<String, Value>,
    actual: Map<String, Value>,
    attestation: ValidatorAttestation,
}

impl GenericVersionedAdapterPredicate {
    fn derived_claim(
        &self,
        artifacts: &[UntrustedArtifactRef],
        artifact_bytes: &[Vec<u8>],
        verifier: &RegistryVerifierBinding,
        repository: &ValidatedArtifactImportRepository<'_>,
    ) -> Result<Value, EvidenceDomainError> {
        if self.kind != "generic_versioned_adapter_predicate" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "adapter_predicate.kind",
            ));
        }
        require_supported_version(&self.version, "adapter_predicate.version")?;
        NamespacedIdentifier::parse(self.observation_type.clone())?;
        AttemptStatus::from_str(&self.outcome)?;
        reject_forbidden_authority_fields(&self.predicate)?;
        reject_forbidden_authority_fields(&self.actual)?;
        self.attestation
            .verify(self, artifacts, artifact_bytes, verifier, repository)?;
        Ok(json!({
            "kind": self.kind,
            "version": self.version,
            "type": self.observation_type,
            "outcome": self.outcome,
            "predicate": self.predicate,
            "actual": self.actual,
            "attestation": self.attestation.to_value(),
        }))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidatorAttestation {
    kind: String,
    version: String,
    artifact_set_digest: String,
    predicate_digest: String,
    verifier_digest: String,
    verifier_instance_digest: String,
    probe_execution_id: String,
    probe_result_digest: String,
    validator_attempt_id: String,
    validator_receipt_id: String,
    validator_receipt_digest: String,
}

impl ValidatorAttestation {
    fn verify(
        &self,
        predicate: &GenericVersionedAdapterPredicate,
        artifacts: &[UntrustedArtifactRef],
        artifact_bytes: &[Vec<u8>],
        verifier: &RegistryVerifierBinding,
        repository: &ValidatedArtifactImportRepository<'_>,
    ) -> Result<(), EvidenceDomainError> {
        if self.kind != "planr_import_validator_attestation" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "adapter_predicate.attestation.kind",
            ));
        }
        require_supported_version(&self.version, "adapter_predicate.attestation.version")?;
        super::model::Sha256Digest::parse(self.artifact_set_digest.clone())?;
        super::model::Sha256Digest::parse(self.predicate_digest.clone())?;
        super::model::Sha256Digest::parse(self.verifier_digest.clone())?;
        super::model::Sha256Digest::parse(self.verifier_instance_digest.clone())?;
        EvidenceId::parse(self.probe_execution_id.clone())?;
        super::model::Sha256Digest::parse(self.probe_result_digest.clone())?;
        EvidenceId::parse(self.validator_attempt_id.clone())?;
        EvidenceId::parse(self.validator_receipt_id.clone())?;
        super::model::Sha256Digest::parse(self.validator_receipt_digest.clone())?;
        let artifact_set_digest = artifact_set_digest(artifacts, artifact_bytes)?;
        let predicate_digest = generic_predicate_digest(predicate)?;
        if self.artifact_set_digest != artifact_set_digest
            || self.predicate_digest != predicate_digest
            || self.verifier_digest != verifier.digest
            || self.verifier_instance_digest != verifier.instance_digest
            || self.probe_execution_id != verifier.probe_execution_id
            || self.probe_result_digest != verifier.probe_result_digest
        {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "adapter_predicate.attestation",
            ));
        }
        verify_persisted_validator_observation(self, verifier, repository)
    }

    fn to_value(&self) -> Value {
        json!({
            "kind": self.kind,
            "version": self.version,
            "artifact_set_digest": self.artifact_set_digest,
            "predicate_digest": self.predicate_digest,
            "verifier_digest": self.verifier_digest,
            "verifier_instance_digest": self.verifier_instance_digest,
            "probe_execution_id": self.probe_execution_id,
            "probe_result_digest": self.probe_result_digest,
            "validator_attempt_id": self.validator_attempt_id,
            "validator_receipt_id": self.validator_receipt_id,
            "validator_receipt_digest": self.validator_receipt_digest,
        })
    }
}

fn verify_persisted_validator_observation(
    attestation: &ValidatorAttestation,
    verifier: &RegistryVerifierBinding,
    repository: &ValidatedArtifactImportRepository<'_>,
) -> Result<(), EvidenceDomainError> {
    let (
        capability_instance_id,
        attempt_obligation_id,
        attempt_stdout_digest,
        attempt_json,
        attempt_status,
        receipt_obligation_id,
        receipt_status,
        receipt_digest,
        receipt_json,
    ): (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) = repository
        .conn
        .query_row(
            "SELECT attempts.capability_instance_id, attempts.obligation_id,
                    attempts.stdout_digest, attempts.attempt_json, attempts.attempt_status,
                    receipts.obligation_id, receipts.receipt_status, receipts.receipt_digest,
                    receipts.receipt_json
             FROM evidence_attempts AS attempts
             JOIN evidence_receipts AS receipts
               ON receipts.attempt_id = attempts.id
              AND receipts.project_id = attempts.project_id
             WHERE attempts.project_id = ?1
               AND attempts.id = ?2
               AND receipts.id = ?3",
            params![
                repository.project_id,
                attestation.validator_attempt_id,
                attestation.validator_receipt_id
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ))?;
    if capability_instance_id != verifier.instance_id
        || attempt_obligation_id != receipt_obligation_id
        || attempt_status != AttemptStatus::Passed.as_str()
        || receipt_status != "trusted"
        || receipt_digest != attestation.validator_receipt_digest
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ));
    }
    let receipt_value: Value = serde_json::from_str(&receipt_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("receipt_json"))?;
    let receipt = EvidenceReceipt::from_trusted_value(receipt_value)?;
    if receipt.receipt_digest().as_str() != receipt_digest
        || receipt.obligation_id().as_str() != attempt_obligation_id
        || receipt.capability().instance_id.as_str() != verifier.instance_id
        || receipt.capability().instance_digest.as_str() != verifier.instance_digest
        || !receipt
            .attempt_ids()
            .iter()
            .any(|id| id.as_str() == attestation.validator_attempt_id)
        || receipt.provenance().execution_id != attestation.validator_attempt_id
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ));
    }
    let Some(observation) = receipt.observations().iter().find(|observation| {
        observation.observation_type.as_str() == "planr.import.validator.generic_predicate"
    }) else {
        return Err(EvidenceDomainError::MissingTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ));
    };
    if observation.outcome != AttemptStatus::Passed {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ));
    }
    let attempt_value: Value = serde_json::from_str(&attempt_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("attempt_json"))?;
    let attempt_raw_result =
        attempt_value
            .get("raw_result")
            .ok_or(EvidenceDomainError::MissingTrustedBinding(
                "attempt_json.raw_result",
            ))?;
    let stdout = validator_stdout_result(
        &observation.actual,
        attempt_raw_result,
        &attempt_stdout_digest,
        receipt.raw_result().digest.as_str(),
    )?;
    if stdout.verdict != AttemptStatus::Passed.as_str()
        || stdout.artifact_set_digest != attestation.artifact_set_digest
        || stdout.predicate_digest != attestation.predicate_digest
        || stdout.verifier_digest != attestation.verifier_digest
        || stdout.verifier_instance_digest != attestation.verifier_instance_digest
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ));
    }
    Ok(())
}

fn validator_stdout_result(
    actual: &Map<String, Value>,
    attempt_raw_result: &Value,
    attempt_stdout_digest: &str,
    receipt_raw_result_digest: &str,
) -> Result<GenericValidatorResult, EvidenceDomainError> {
    let process_result =
        attempt_raw_result
            .as_object()
            .ok_or(EvidenceDomainError::InvalidTrustedBinding(
                "adapter_predicate.trusted_validator_observation.attempt.raw_result",
            ))?;
    if process_result.get("kind").and_then(Value::as_str) != Some("process_output")
        || process_result
            .get("exit")
            .and_then(Value::as_object)
            .and_then(|exit| exit.get("exit_code"))
            .and_then(Value::as_i64)
            != Some(0)
        || process_result
            .get("stdout_truncated")
            .and_then(Value::as_bool)
            != Some(false)
        || process_result
            .get("stderr_truncated")
            .and_then(Value::as_bool)
            != Some(false)
        || process_result.get("stderr_bytes").and_then(Value::as_u64) != Some(0)
        || process_result.get("stdout_digest").and_then(Value::as_str)
            != Some(attempt_stdout_digest)
        || process_result.get("stderr_digest").and_then(Value::as_str)
            != Some("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.attempt.raw_result",
        ));
    }
    let actual_raw_result_digest = sha256_json_digest(attempt_raw_result)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    if actual_raw_result_digest != receipt_raw_result_digest {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.attempt.raw_result_digest",
        ));
    }
    let stdout_excerpt = process_result
        .get("stdout_excerpt")
        .and_then(Value::as_str)
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "adapter_predicate.trusted_validator_observation.attempt.stdout",
        ))?;
    let stdout_bytes = stdout_excerpt.as_bytes();
    if process_result.get("stdout_bytes").and_then(Value::as_u64) != Some(stdout_bytes.len() as u64)
        || sha256_prefixed_bytes(stdout_bytes) != attempt_stdout_digest
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.attempt.stdout",
        ));
    }
    let parsed: Value = serde_json::from_str(stdout_excerpt).map_err(|_| {
        EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.attempt.stdout",
        )
    })?;
    let mut observed = actual.clone();
    if observed
        .get("schema_ref")
        .is_some_and(|value| !value.is_string())
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.actual",
        ));
    }
    observed.remove("schema_ref");
    if Value::Object(observed) != parsed {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.actual",
        ));
    }
    let result: GenericValidatorResult = serde_json::from_value(parsed).map_err(|_| {
        EvidenceDomainError::InvalidTrustedBinding(
            "adapter_predicate.trusted_validator_observation.actual",
        )
    })?;
    result.validate()?;
    Ok(result)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericValidatorResult {
    kind: String,
    version: String,
    verdict: String,
    artifact_set_digest: String,
    predicate_digest: String,
    verifier_digest: String,
    verifier_instance_digest: String,
}

impl GenericValidatorResult {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        if self.kind != "planr.import.validator.generic_predicate.result" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "adapter_predicate.trusted_validator_observation.actual.stdout.kind",
            ));
        }
        require_supported_version(
            &self.version,
            "adapter_predicate.trusted_validator_observation.actual.stdout.version",
        )?;
        AttemptStatus::from_str(&self.verdict)?;
        super::model::Sha256Digest::parse(self.artifact_set_digest.clone())?;
        super::model::Sha256Digest::parse(self.predicate_digest.clone())?;
        super::model::Sha256Digest::parse(self.verifier_digest.clone())?;
        super::model::Sha256Digest::parse(self.verifier_instance_digest.clone())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanrRunnerResult {
    kind: String,
    version: String,
    command: Vec<String>,
    exit_code: i64,
    status: String,
    stdout_digest: String,
    stderr_digest: String,
    duration_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanrRunnerArtifact {
    command: Vec<String>,
    exit_code: i64,
    status: String,
    stdout_digest: String,
    stderr_digest: String,
    duration_ms: u64,
}

impl PlanrRunnerResult {
    fn derived_claim(
        &self,
        artifacts: &[UntrustedArtifactRef],
        artifact_bytes: &[Vec<u8>],
    ) -> Result<Value, EvidenceDomainError> {
        if self.kind != "planr_runner_result" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "runner_result.kind",
            ));
        }
        require_supported_version(&self.version, "runner_result.version")?;
        if self.command.is_empty() || self.command.iter().any(|part| part.is_empty()) {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "runner_result.command",
            ));
        }
        let status = AttemptStatus::from_str(&self.status)?;
        match status {
            AttemptStatus::Passed if self.exit_code == 0 => {}
            AttemptStatus::Failed if self.exit_code > 0 => {}
            _ => {
                return Err(EvidenceDomainError::InvalidTrustedBinding(
                    "runner_result.exit_code",
                ));
            }
        }
        let artifact = exact_artifact_bytes_for_kind(artifacts, artifact_bytes, "runner-json")?;
        let observed: PlanrRunnerArtifact = serde_json::from_slice(artifact)
            .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("runner_result.artifact"))?;
        if observed.command != self.command
            || observed.exit_code != self.exit_code
            || observed.status != self.status
            || observed.stdout_digest != self.stdout_digest
            || observed.stderr_digest != self.stderr_digest
            || observed.duration_ms != self.duration_ms
        {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "runner_result.artifact",
            ));
        }
        super::model::Sha256Digest::parse(self.stdout_digest.clone())?;
        super::model::Sha256Digest::parse(self.stderr_digest.clone())?;
        Ok(json!({
            "kind": self.kind,
            "version": self.version,
            "command": self.command,
            "exit_code": self.exit_code,
            "status": self.status,
            "stdout_digest": self.stdout_digest,
            "stderr_digest": self.stderr_digest,
            "duration_ms": self.duration_ms,
            "artifact_digest": sha256_prefixed_bytes(artifact),
        }))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct JunitXmlEvidence {
    kind: String,
    version: String,
}

impl JunitXmlEvidence {
    fn derived_claim(
        &self,
        artifacts: &[UntrustedArtifactRef],
        artifact_bytes: &[Vec<u8>],
    ) -> Result<Value, EvidenceDomainError> {
        if self.kind != "junit_xml" {
            return Err(EvidenceDomainError::InvalidTrustedBinding("junit.kind"));
        }
        require_supported_version(&self.version, "junit.version")?;
        let artifact = exact_artifact_bytes_for_kind(artifacts, artifact_bytes, "junit-xml")?;
        let content = std::str::from_utf8(artifact)
            .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
        let counts = parse_junit_counts(content)?;
        let outcome = if counts.failures > 0 || counts.errors > 0 {
            AttemptStatus::Failed.as_str()
        } else if counts.tests == 0 {
            AttemptStatus::Inconclusive.as_str()
        } else if counts.tests == counts.skipped {
            AttemptStatus::Skipped.as_str()
        } else {
            AttemptStatus::Passed.as_str()
        };
        Ok(json!({
            "kind": self.kind,
            "version": self.version,
            "outcome": outcome,
            "tests": counts.tests,
            "failures": counts.failures,
            "errors": counts.errors,
            "skipped": counts.skipped,
            "artifact_digest": sha256_prefixed_bytes(artifact),
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidatedArtifactImportRaw {
    id: String,
    schema_version: String,
    #[serde(default, deserialize_with = "deserialize_optional_string_without_null")]
    source_kind: Option<String>,
    submitted_at: String,
    format: String,
    verifier_identity: VerifierIdentity,
    #[serde(default, deserialize_with = "deserialize_optional_value_without_null")]
    adapter_predicate: Option<GenericVersionedAdapterPredicate>,
    #[serde(default, deserialize_with = "deserialize_optional_value_without_null")]
    runner_result: Option<PlanrRunnerResult>,
    #[serde(default, deserialize_with = "deserialize_optional_value_without_null")]
    junit: Option<JunitXmlEvidence>,
    artifact_refs: Vec<UntrustedArtifactRef>,
    producer_metadata: Map<String, Value>,
}

impl ValidatedArtifactImportRaw {
    fn into_import(self) -> Result<ValidatedArtifactImport, EvidenceDomainError> {
        if self.schema_version != VALIDATED_ARTIFACT_IMPORT_SCHEMA_VERSION {
            return Err(EvidenceDomainError::InvalidSchemaVersion(
                self.schema_version,
            ));
        }
        if let Some(source_kind) = self.source_kind {
            if source_kind != SourceKind::ArtifactImport.as_str() {
                return Err(EvidenceDomainError::InvalidStatus {
                    kind: "validated artifact import source_kind",
                    value: source_kind,
                });
            }
        }
        EvidenceId::parse(self.id.clone())?;
        super::model::validate_timestamp(&self.submitted_at)?;
        self.verifier_identity.validate()?;
        if self.artifact_refs.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding("artifact_refs"));
        }
        reject_forbidden_authority_fields(&self.producer_metadata)?;

        let evidence = match self.format.as_str() {
            GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT => {
                ensure_no_extra_format_fields(self.runner_result.is_some(), self.junit.is_some())?;
                ImportEvidence::Generic(Box::new(self.adapter_predicate.ok_or(
                    EvidenceDomainError::MissingTrustedBinding("adapter_predicate"),
                )?))
            }
            PLANR_RUNNER_RESULT_FORMAT => {
                ensure_no_extra_format_fields(
                    self.adapter_predicate.is_some(),
                    self.junit.is_some(),
                )?;
                ImportEvidence::Runner(
                    self.runner_result
                        .ok_or(EvidenceDomainError::MissingTrustedBinding("runner_result"))?,
                )
            }
            JUNIT_XML_FORMAT => {
                ensure_no_extra_format_fields(
                    self.adapter_predicate.is_some(),
                    self.runner_result.is_some(),
                )?;
                ImportEvidence::Junit(
                    self.junit
                        .ok_or(EvidenceDomainError::MissingTrustedBinding("junit"))?,
                )
            }
            _ => return Err(EvidenceDomainError::InvalidTrustedBinding("format")),
        };

        Ok(ValidatedArtifactImport {
            id: self.id,
            submitted_at: self.submitted_at,
            artifact_refs: self.artifact_refs,
            verifier_identity: self.verifier_identity,
            evidence,
            producer_metadata: self.producer_metadata,
            format: self.format,
        })
    }
}

pub fn parse_validated_artifact_import(
    value: Value,
    repository: &ValidatedArtifactImportRepository<'_>,
) -> Result<ValidatedImportRecord, serde_json::Error> {
    reject_forbidden_authority_value(&value).map_err(json_error)?;
    let raw = serde_json::from_value::<ValidatedArtifactImportRaw>(value)?;
    let import = raw.into_import().map_err(json_error)?;
    let artifact_bytes =
        validate_artifact_bindings(&import.artifact_refs, repository).map_err(json_error)?;
    let verifier = bind_registered_verifier(&import, repository).map_err(json_error)?;
    let claims = import
        .evidence
        .claims(
            &import.artifact_refs,
            &artifact_bytes,
            &verifier,
            repository,
        )
        .map_err(json_error)?;
    let proposal = build_proposal(import, claims, &verifier).map_err(json_error)?;
    persist_validated_import(repository, proposal).map_err(json_error)
}

fn build_proposal(
    import: ValidatedArtifactImport,
    mut claims: Map<String, Value>,
    verifier: &RegistryVerifierBinding,
) -> Result<UntrustedEvidenceProposal, EvidenceDomainError> {
    let artifact_digests = import
        .artifact_refs
        .iter()
        .map(|artifact| {
            json!({
                "id": artifact.id,
                "kind": artifact.kind,
                "digest": artifact.digest,
            })
        })
        .collect::<Vec<_>>();
    claims.insert(
        "artifact_digests".to_string(),
        Value::Array(artifact_digests),
    );

    let mut producer_metadata = import.producer_metadata;
    producer_metadata.insert(
        "validated_import".to_string(),
        json!({
            "schema_version": VALIDATED_ARTIFACT_IMPORT_SCHEMA_VERSION,
            "format": import.format,
            "verifier_identity": import.verifier_identity.to_value(),
        }),
    );
    producer_metadata.insert("registered_verifier".to_string(), verifier.to_value());

    let proposal = UntrustedEvidenceProposal {
        id: import.id,
        schema_version: super::model::EVIDENCE_CONTRACT_V1.to_string(),
        source_kind: SourceKind::ArtifactImport.as_str().to_string(),
        submitted_at: import.submitted_at,
        claims,
        artifact_refs: import.artifact_refs,
        producer_metadata,
    };
    proposal.validate()?;
    Ok(proposal)
}

fn bind_registered_verifier(
    import: &ValidatedArtifactImport,
    repository: &ValidatedArtifactImportRepository<'_>,
) -> Result<RegistryVerifierBinding, EvidenceDomainError> {
    let required_instance_id = required_capability_instance_id(import, repository)?;
    let mut statement = repository
        .conn
        .prepare(
            "SELECT manifests.id, manifests.version, manifests.adapter_digest,
                    manifests.manifest_digest, manifests.manifest_json,
                    instances.id, instances.probe_execution_id, instances.runtime_target_json,
                    instances.host_fingerprint_json, instances.capability_snapshot_json,
                    instances.probe_result_json
             FROM verification_capability_manifests AS manifests
             JOIN verification_capability_instances AS instances
               ON instances.manifest_id = manifests.id
              AND instances.manifest_version = manifests.version
              AND instances.manifest_digest = manifests.manifest_digest
             WHERE manifests.id = ?1 AND manifests.version = ?2
               AND instances.availability_status = 'available'
               AND (?3 IS NULL OR instances.id = ?3)
             ORDER BY instances.created_at DESC",
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let rows = statement
        .query_map(
            params![
                import.verifier_identity.id,
                import.verifier_identity.version,
                required_instance_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let row = match rows.as_slice() {
        [row] => row.clone(),
        [] => {
            return Err(EvidenceDomainError::MissingTrustedBinding(
                "registered_verifier",
            ));
        }
        _ => {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "registered_verifier.instance",
            ));
        }
    };
    let (
        manifest_id,
        version,
        adapter_digest,
        manifest_digest,
        manifest_json,
        instance_id,
        probe_execution_id,
        runtime_target_json,
        host_fingerprint_json,
        capability_snapshot_json,
        probe_result_json,
    ) = row;
    if adapter_digest != import.verifier_identity.digest {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "registered_verifier",
        ));
    }
    let manifest_value: Value = serde_json::from_str(&manifest_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("manifest_json"))?;
    let actual_manifest_digest = sha256_json_digest(&manifest_value)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    if actual_manifest_digest != manifest_digest {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "manifest_digest",
        ));
    }
    let manifest: VerificationCapabilityManifest = serde_json::from_value(manifest_value)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("manifest_json"))?;
    let instance_value: Value = serde_json::from_str(&capability_snapshot_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("capability_snapshot_json"))?;
    let instance: VerificationCapabilityInstance =
        serde_json::from_value(instance_value.clone())
            .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("capability_snapshot_json"))?;
    let instance_digest = sha256_json_digest(&instance_value)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let runtime_target_value: Value = serde_json::from_str(&runtime_target_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("runtime_target_json"))?;
    let host_fingerprint_value: Value = serde_json::from_str(&host_fingerprint_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("host_fingerprint_json"))?;
    let probe_result_value: Value = serde_json::from_str(&probe_result_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("probe_result_json"))?;
    let probe_result_digest = sha256_json_digest(&probe_result_value)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    if manifest.id.as_str() != manifest_id
        || manifest.version != version
        || manifest.adapter_digest.as_str() != adapter_digest
        || import.verifier_identity.kind != SourceKind::Adapter.as_str()
        || import.verifier_identity.name != manifest.id.as_str()
        || instance.id.as_str() != instance_id
        || instance.manifest_id.as_str() != manifest_id
        || instance.manifest_digest.as_str() != manifest_digest
        || instance.adapter_version != manifest.version
        || instance.probe_result.probe_execution_id.as_str() != probe_execution_id
        || instance.availability.status != CapabilityAvailabilityStatus::Available
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "registered_verifier",
        ));
    }
    require_runtime_binding_matches(
        &manifest,
        &instance,
        &runtime_target_value,
        &host_fingerprint_value,
        import,
    )?;
    require_manifest_authorizes_import(&manifest, import)?;
    Ok(RegistryVerifierBinding {
        kind: SourceKind::Adapter.as_str().to_string(),
        id: manifest_id.clone(),
        name: manifest.id.as_str().to_string(),
        version,
        digest: adapter_digest,
        manifest_id,
        manifest_digest,
        instance_id,
        instance_digest,
        probe_execution_id,
        probe_result_digest,
    })
}

fn required_capability_instance_id(
    import: &ValidatedArtifactImport,
    repository: &ValidatedArtifactImportRepository<'_>,
) -> Result<Option<String>, EvidenceDomainError> {
    let ImportEvidence::Generic(predicate) = &import.evidence else {
        return Ok(None);
    };
    let instance_id = repository
        .conn
        .query_row(
            "SELECT attempts.capability_instance_id
             FROM evidence_attempts AS attempts
             JOIN evidence_receipts AS receipts
               ON receipts.attempt_id = attempts.id
              AND receipts.project_id = attempts.project_id
             WHERE attempts.project_id = ?1
               AND attempts.id = ?2
               AND receipts.id = ?3",
            params![
                repository.project_id,
                predicate.attestation.validator_attempt_id,
                predicate.attestation.validator_receipt_id
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?
        .ok_or(EvidenceDomainError::MissingTrustedBinding(
            "adapter_predicate.trusted_validator_observation",
        ))?;
    Ok(Some(instance_id))
}

fn require_runtime_binding_matches(
    manifest: &VerificationCapabilityManifest,
    instance: &VerificationCapabilityInstance,
    runtime_target_value: &Value,
    host_fingerprint_value: &Value,
    import: &ValidatedArtifactImport,
) -> Result<(), EvidenceDomainError> {
    let manifest_runtime_targets = serde_json::to_value(&manifest.runtime_targets)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    if runtime_target_value != &manifest_runtime_targets {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "runtime_target_json",
        ));
    }
    if !manifest
        .supported_surfaces
        .iter()
        .any(|surface| surface == &instance.surface)
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "instance.surface",
        ));
    }
    if host_fingerprint_value.get("environment")
        != Some(
            &serde_json::to_value(&instance.environment)
                .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?,
        )
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "host_fingerprint_json",
        ));
    }
    let observation = import.evidence.observation_type();
    if !instance
        .observed_payload_contract
        .observation_types
        .iter()
        .any(|observed| observed.as_str() == observation)
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "instance.observed_payload_contract",
        ));
    }
    let Some(manifest_binding) = manifest
        .supported_observations
        .iter()
        .find(|binding| binding.observation_type.as_str() == observation)
    else {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "manifest.supported_observations",
        ));
    };
    if instance.observed_payload_contract.schema_ref != manifest_binding.schema_ref {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "instance.observed_payload_contract.schema_ref",
        ));
    }
    Ok(())
}

fn require_manifest_authorizes_import(
    manifest: &VerificationCapabilityManifest,
    import: &ValidatedArtifactImport,
) -> Result<(), EvidenceDomainError> {
    if manifest.adapter_kind != AdapterKind::ArtifactImport
        || manifest.provenance_path != ProvenanceSourceKind::ValidatedArtifactImport
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding("manifest_json"));
    }
    if !manifest
        .supported_artifacts
        .iter()
        .any(|format| format == &import.format)
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "manifest.supported_artifacts",
        ));
    }
    let observation = import.evidence.observation_type();
    if !manifest
        .supported_observations
        .iter()
        .any(|binding| binding.observation_type.as_str() == observation)
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "manifest.supported_observations",
        ));
    }
    Ok(())
}

fn validate_artifact_bindings(
    artifact_refs: &[UntrustedArtifactRef],
    repository: &ValidatedArtifactImportRepository<'_>,
) -> Result<Vec<Vec<u8>>, EvidenceDomainError> {
    let mut bytes = Vec::with_capacity(artifact_refs.len());
    for artifact_ref in artifact_refs {
        reject_forbidden_authority_fields(&artifact_ref.extra)?;
        let uri = artifact_ref
            .uri
            .as_deref()
            .ok_or(EvidenceDomainError::MissingTrustedBinding(
                "artifact_refs[].uri",
            ))?;
        let path = contained_artifact_path(repository.artifact_root, uri)?;
        let content = fs::read(&path).map_err(|err| {
            EvidenceDomainError::Digest(format!(
                "failed to read artifact {}: {err}",
                path.display()
            ))
        })?;
        let actual_digest = sha256_prefixed_bytes(&content);
        if actual_digest != artifact_ref.digest {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "artifact_refs[].digest",
            ));
        }
        bytes.push(content);
    }
    Ok(bytes)
}

fn persist_validated_import(
    repository: &ValidatedArtifactImportRepository<'_>,
    proposal: UntrustedEvidenceProposal,
) -> Result<ValidatedImportRecord, EvidenceDomainError> {
    let proposal_value = proposal_identity_value(&proposal);
    let digest = sha256_json_digest(&proposal_value)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let proposal_json = serde_json::to_string(&proposal_value)
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    repository
        .conn
        .execute(
            "INSERT OR IGNORE INTO evidence_validated_imports(
              project_id, id, digest, proposal_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![repository.project_id, proposal.id, digest, proposal_json],
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let existing_digest: String = repository
        .conn
        .query_row(
            "SELECT digest FROM evidence_validated_imports WHERE project_id = ?1 AND id = ?2",
            params![repository.project_id, proposal.id],
            |row| row.get(0),
        )
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    if existing_digest == digest {
        let idempotent = repository
            .conn
            .query_row("SELECT changes() = 0", [], |row| row.get::<_, bool>(0))
            .unwrap_or(false);
        Ok(ValidatedImportRecord {
            id: proposal.id.clone(),
            digest,
            idempotent,
            proposal,
        })
    } else {
        Err(EvidenceDomainError::InvalidTrustedBinding("import.id"))
    }
}

fn contained_artifact_path(root: &Path, uri: &str) -> Result<PathBuf, EvidenceDomainError> {
    let relative = uri.strip_prefix("file://").unwrap_or(uri);
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "artifact_refs[].uri",
        ));
    }
    let root = root.canonicalize().map_err(|err| {
        EvidenceDomainError::Digest(format!(
            "failed to resolve artifact root {}: {err}",
            root.display()
        ))
    })?;
    let path = root.join(candidate).canonicalize().map_err(|err| {
        EvidenceDomainError::Digest(format!("failed to resolve artifact {uri}: {err}"))
    })?;
    if !path.starts_with(&root) {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "artifact_refs[].uri",
        ));
    }
    Ok(path)
}

fn exact_artifact_bytes_for_kind<'a>(
    artifacts: &[UntrustedArtifactRef],
    bytes: &'a [Vec<u8>],
    kind: &str,
) -> Result<&'a [u8], EvidenceDomainError> {
    if artifacts.len() != 1 {
        return Err(EvidenceDomainError::InvalidTrustedBinding("artifact_refs"));
    }
    let artifact = artifacts
        .first()
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("artifact_refs"))?;
    if artifact.kind != kind {
        return Err(EvidenceDomainError::InvalidTrustedBinding(
            "artifact_refs[].kind",
        ));
    }
    bytes
        .first()
        .map(Vec::as_slice)
        .ok_or(EvidenceDomainError::InvalidTrustedBinding(
            "artifact_refs[].kind",
        ))
}

#[derive(Debug, Clone, Copy)]
struct JunitCounts {
    tests: u64,
    failures: u64,
    errors: u64,
    skipped: u64,
}

#[derive(Debug, Clone, Copy)]
struct JunitFrame {
    declared_tests: Option<u64>,
    declared_failures: Option<u64>,
    declared_errors: Option<u64>,
    declared_skipped: Option<u64>,
    counts: JunitCounts,
}

#[derive(Debug, Clone, Copy, Default)]
struct JunitAttributes {
    tests: Option<u64>,
    failures: Option<u64>,
    errors: Option<u64>,
    skipped: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct JunitTestcaseState {
    terminal: Option<JunitElement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JunitElement {
    TestSuites,
    TestSuite,
    TestCase,
    Failure,
    Error,
    Skipped,
    SystemOut,
    SystemErr,
    Properties,
    Property,
}

const JUNIT_MAX_BYTES: usize = 8 * 1024 * 1024;
const JUNIT_MAX_DEPTH: usize = 128;
const JUNIT_MAX_EVENTS: usize = 1_000_000;

fn parse_junit_counts(content: &str) -> Result<JunitCounts, EvidenceDomainError> {
    if content.len() > JUNIT_MAX_BYTES {
        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
    }
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut counts: Option<JunitCounts> = None;
    let mut frames: Vec<JunitFrame> = Vec::new();
    let mut element_stack: Vec<JunitElement> = Vec::new();
    let mut testcase_stack: Vec<JunitTestcaseState> = Vec::new();
    let mut depth = 0usize;
    let mut events = 0usize;
    let mut root: Option<Vec<u8>> = None;
    let mut closed_root = false;
    let mut saw_suite = false;

    loop {
        events += 1;
        if events > JUNIT_MAX_EVENTS {
            return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
        }
        match reader
            .read_event_into(&mut buf)
            .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?
        {
            Event::Start(event) => {
                if closed_root {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
                let name = event.name().as_ref().to_vec();
                let element = junit_element(&name)?;
                let attributes = junit_attributes(&event)?;
                if depth == 0 {
                    if !matches!(name.as_slice(), b"testsuite" | b"testsuites") || root.is_some() {
                        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                    }
                    root = Some(name.clone());
                } else {
                    validate_junit_child(element_stack.last().copied(), element)?;
                }
                depth += 1;
                if depth > JUNIT_MAX_DEPTH {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
                element_stack.push(element);
                if element == JunitElement::TestSuite {
                    saw_suite = true;
                }
                if matches!(element, JunitElement::TestSuite | JunitElement::TestSuites) {
                    frames.push(junit_frame(attributes));
                } else if element == JunitElement::TestCase {
                    add_junit_testcase(&mut frames)?;
                    testcase_stack.push(JunitTestcaseState::default());
                } else if element == JunitElement::Failure {
                    mark_junit_terminal(&mut frames, &mut testcase_stack, JunitElement::Failure)?;
                } else if element == JunitElement::Error {
                    mark_junit_terminal(&mut frames, &mut testcase_stack, JunitElement::Error)?;
                } else if element == JunitElement::Skipped {
                    mark_junit_terminal(&mut frames, &mut testcase_stack, JunitElement::Skipped)?;
                }
            }
            Event::Empty(event) => {
                if closed_root {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
                let name = event.name().as_ref().to_vec();
                let element = junit_element(&name)?;
                let attributes = junit_attributes(&event)?;
                if depth == 0 {
                    if root.is_some() || name.as_slice() != b"testsuite" {
                        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                    }
                    root = Some(name.clone());
                    closed_root = true;
                } else {
                    validate_junit_child(element_stack.last().copied(), element)?;
                }
                if element == JunitElement::TestSuite {
                    saw_suite = true;
                }
                if matches!(element, JunitElement::TestSuite | JunitElement::TestSuites) {
                    let frame = junit_frame(attributes);
                    let frame_counts = validate_junit_frame(frame)?;
                    add_junit_counts_to_parent(&mut frames, frame_counts, &mut counts)?;
                } else if element == JunitElement::TestCase {
                    add_junit_testcase(&mut frames)?;
                } else if element == JunitElement::Failure {
                    mark_junit_terminal(&mut frames, &mut testcase_stack, JunitElement::Failure)?;
                } else if element == JunitElement::Error {
                    mark_junit_terminal(&mut frames, &mut testcase_stack, JunitElement::Error)?;
                } else if element == JunitElement::Skipped {
                    mark_junit_terminal(&mut frames, &mut testcase_stack, JunitElement::Skipped)?;
                }
            }
            Event::End(event) => {
                if depth == 0 {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
                let name = event.name().as_ref().to_vec();
                let element = junit_element(&name)?;
                if element_stack.pop() != Some(element) {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
                if element == JunitElement::TestCase && testcase_stack.pop().is_none() {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
                if matches!(element, JunitElement::TestSuite | JunitElement::TestSuites) {
                    let frame = frames
                        .pop()
                        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
                    let frame_counts = validate_junit_frame(frame)?;
                    add_junit_counts_to_parent(&mut frames, frame_counts, &mut counts)?;
                }
                depth -= 1;
                if depth == 0 {
                    closed_root = true;
                }
            }
            Event::Text(text) => {
                if depth == 0 && !text_is_whitespace(text.as_ref()) {
                    return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
                }
            }
            Event::Decl(_)
            | Event::Comment(_)
            | Event::CData(_)
            | Event::PI(_)
            | Event::DocType(_) => {}
            Event::GeneralRef(_) => {
                return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
            }
            Event::Eof => break,
        }
        buf.clear();
    }
    if depth != 0
        || !element_stack.is_empty()
        || !testcase_stack.is_empty()
        || !closed_root
        || !saw_suite
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
    }
    counts.ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))
}

fn junit_element(name: &[u8]) -> Result<JunitElement, EvidenceDomainError> {
    match name {
        b"testsuites" => Ok(JunitElement::TestSuites),
        b"testsuite" => Ok(JunitElement::TestSuite),
        b"testcase" => Ok(JunitElement::TestCase),
        b"failure" => Ok(JunitElement::Failure),
        b"error" => Ok(JunitElement::Error),
        b"skipped" => Ok(JunitElement::Skipped),
        b"system-out" => Ok(JunitElement::SystemOut),
        b"system-err" => Ok(JunitElement::SystemErr),
        b"properties" => Ok(JunitElement::Properties),
        b"property" => Ok(JunitElement::Property),
        _ => Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact")),
    }
}

fn validate_junit_child(
    parent: Option<JunitElement>,
    child: JunitElement,
) -> Result<(), EvidenceDomainError> {
    let valid = matches!(
        (parent, child),
        (
            Some(JunitElement::TestSuites),
            JunitElement::TestSuite | JunitElement::Properties
        ) | (
            Some(JunitElement::TestSuite),
            JunitElement::TestSuite
                | JunitElement::TestCase
                | JunitElement::Properties
                | JunitElement::SystemOut
                | JunitElement::SystemErr
        ) | (
            Some(JunitElement::TestCase),
            JunitElement::Failure
                | JunitElement::Error
                | JunitElement::Skipped
                | JunitElement::SystemOut
                | JunitElement::SystemErr
        ) | (Some(JunitElement::Properties), JunitElement::Property)
    );
    if valid {
        Ok(())
    } else {
        Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))
    }
}

fn junit_frame(attributes: JunitAttributes) -> JunitFrame {
    JunitFrame {
        declared_tests: attributes.tests,
        declared_failures: attributes.failures,
        declared_errors: attributes.errors,
        declared_skipped: attributes.skipped,
        counts: JunitCounts {
            tests: 0,
            failures: 0,
            errors: 0,
            skipped: 0,
        },
    }
}

fn validate_junit_frame(frame: JunitFrame) -> Result<JunitCounts, EvidenceDomainError> {
    if frame.counts.skipped > frame.counts.tests {
        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
    }
    if frame
        .declared_tests
        .is_some_and(|declared| declared != frame.counts.tests)
        || frame
            .declared_failures
            .is_some_and(|declared| declared != frame.counts.failures)
        || frame
            .declared_errors
            .is_some_and(|declared| declared != frame.counts.errors)
        || frame
            .declared_skipped
            .is_some_and(|declared| declared != frame.counts.skipped)
    {
        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
    }
    Ok(frame.counts)
}

fn add_junit_counts_to_parent(
    frames: &mut [JunitFrame],
    counts: JunitCounts,
    root_counts: &mut Option<JunitCounts>,
) -> Result<(), EvidenceDomainError> {
    if let Some(parent) = frames.last_mut() {
        parent.counts = add_junit_counts(parent.counts, counts)?;
    } else if root_counts.replace(counts).is_some() {
        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
    }
    Ok(())
}

fn add_junit_counts(
    left: JunitCounts,
    right: JunitCounts,
) -> Result<JunitCounts, EvidenceDomainError> {
    Ok(JunitCounts {
        tests: left
            .tests
            .checked_add(right.tests)
            .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?,
        failures: left
            .failures
            .checked_add(right.failures)
            .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?,
        errors: left
            .errors
            .checked_add(right.errors)
            .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?,
        skipped: left
            .skipped
            .checked_add(right.skipped)
            .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?,
    })
}

fn add_junit_testcase(frames: &mut [JunitFrame]) -> Result<(), EvidenceDomainError> {
    let frame = frames
        .last_mut()
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    frame.counts.tests = frame
        .counts
        .tests
        .checked_add(1)
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    Ok(())
}

fn mark_junit_terminal(
    frames: &mut [JunitFrame],
    testcase_stack: &mut [JunitTestcaseState],
    terminal: JunitElement,
) -> Result<(), EvidenceDomainError> {
    let testcase = testcase_stack
        .last_mut()
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    if testcase.terminal.replace(terminal).is_some() {
        return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
    }
    match terminal {
        JunitElement::Failure => add_junit_failure(frames),
        JunitElement::Error => add_junit_error(frames),
        JunitElement::Skipped => add_junit_skipped(frames),
        _ => Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact")),
    }
}

fn add_junit_failure(frames: &mut [JunitFrame]) -> Result<(), EvidenceDomainError> {
    let frame = frames
        .last_mut()
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    frame.counts.failures = frame
        .counts
        .failures
        .checked_add(1)
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    Ok(())
}

fn add_junit_error(frames: &mut [JunitFrame]) -> Result<(), EvidenceDomainError> {
    let frame = frames
        .last_mut()
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    frame.counts.errors = frame
        .counts
        .errors
        .checked_add(1)
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    Ok(())
}

fn add_junit_skipped(frames: &mut [JunitFrame]) -> Result<(), EvidenceDomainError> {
    let frame = frames
        .last_mut()
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    frame.counts.skipped = frame
        .counts
        .skipped
        .checked_add(1)
        .ok_or(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
    Ok(())
}

fn junit_attributes(
    event: &quick_xml::events::BytesStart<'_>,
) -> Result<JunitAttributes, EvidenceDomainError> {
    let mut seen = BTreeSet::new();
    let mut parsed = JunitAttributes::default();
    for attr in event.attributes().with_checks(true) {
        let attr =
            attr.map_err(|_| EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
        let key = attr.key.as_ref().to_vec();
        if !seen.insert(key.clone()) {
            return Err(EvidenceDomainError::InvalidTrustedBinding("junit.artifact"));
        }
        let count = match key.as_slice() {
            b"tests" | b"failures" | b"errors" | b"skipped" => {
                let value = std::str::from_utf8(attr.value.as_ref())
                    .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("junit.artifact"))?;
                Some(
                    value.parse::<u64>().map_err(|_| {
                        EvidenceDomainError::InvalidTrustedBinding("junit.artifact")
                    })?,
                )
            }
            _ => None,
        };
        match (key.as_slice(), count) {
            (b"tests", Some(value)) => parsed.tests = Some(value),
            (b"failures", Some(value)) => parsed.failures = Some(value),
            (b"errors", Some(value)) => parsed.errors = Some(value),
            (b"skipped", Some(value)) => parsed.skipped = Some(value),
            _ => {}
        }
    }
    Ok(parsed)
}

fn text_is_whitespace(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
}

fn artifact_set_digest(
    artifacts: &[UntrustedArtifactRef],
    artifact_bytes: &[Vec<u8>],
) -> Result<String, EvidenceDomainError> {
    let value = Value::Array(
        artifacts
            .iter()
            .zip(artifact_bytes)
            .map(|(artifact, bytes)| {
                json!({
                    "id": artifact.id,
                    "kind": artifact.kind,
                    "declared_digest": artifact.digest,
                    "content_digest": sha256_prefixed_bytes(bytes),
                })
            })
            .collect(),
    );
    sha256_json_digest(&value).map_err(|err| EvidenceDomainError::Digest(err.to_string()))
}

fn generic_predicate_digest(
    predicate: &GenericVersionedAdapterPredicate,
) -> Result<String, EvidenceDomainError> {
    sha256_json_digest(&json!({
        "kind": predicate.kind,
        "version": predicate.version,
        "type": predicate.observation_type,
        "outcome": predicate.outcome,
        "predicate": predicate.predicate,
        "actual": predicate.actual,
    }))
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))
}

fn ensure_no_extra_format_fields(left: bool, right: bool) -> Result<(), EvidenceDomainError> {
    if left || right {
        Err(EvidenceDomainError::InvalidTrustedBinding("format"))
    } else {
        Ok(())
    }
}

fn proposal_identity_value(proposal: &UntrustedEvidenceProposal) -> Value {
    json!({
        "id": proposal.id,
        "schema_version": proposal.schema_version,
        "source_kind": proposal.source_kind,
        "submitted_at": proposal.submitted_at,
        "claims": proposal.claims,
        "artifact_refs": proposal.artifact_refs.iter().map(|artifact| json!({
            "id": artifact.id,
            "kind": artifact.kind,
            "digest": artifact.digest,
            "uri": artifact.uri,
            "extra": artifact.extra,
        })).collect::<Vec<_>>(),
        "producer_metadata": proposal.producer_metadata,
    })
}

fn require_import_field(value: &str, field: &'static str) -> Result<(), EvidenceDomainError> {
    if value.is_empty() {
        Err(EvidenceDomainError::InvalidTrustedBinding(field))
    } else {
        Ok(())
    }
}

fn require_supported_version(value: &str, field: &'static str) -> Result<(), EvidenceDomainError> {
    if value == SUPPORTED_INNER_VERSION {
        Ok(())
    } else {
        Err(EvidenceDomainError::InvalidTrustedBinding(field))
    }
}

fn json_error(error: impl fmt::Display) -> serde_json::Error {
    <serde_json::Error as de::Error>::custom(error.to_string())
}

fn deserialize_optional_string_without_null<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Null => Err(de::Error::custom(
            "optional string field must be omitted or a string, not null",
        )),
        value => serde_json::from_value::<String>(value)
            .map(Some)
            .map_err(de::Error::custom),
    }
}

fn deserialize_optional_value_without_null<'de, D, T>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: for<'a> Deserialize<'a>,
{
    match Value::deserialize(deserializer)? {
        Value::Null => Err(de::Error::custom(
            "optional field must be omitted or a schema-valid value, not null",
        )),
        value => serde_json::from_value::<T>(value)
            .map(Some)
            .map_err(de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::super::execution::{ConfiguredProcessRunInput, run_configured_process_adapter};
    use super::super::model::{
        FixtureDisclosure, ProcessExecutionContract, ProofObligation, TargetBinding,
    };
    use super::*;
    use crate::execution::CancellationToken;
    use rusqlite::Connection;
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Barrier},
        thread,
    };
    use tempfile::TempDir;

    fn digest(content: &str) -> String {
        sha256_prefixed_bytes(content.as_bytes())
    }

    fn repository_policy_yaml() -> String {
        let payload_schema = json!({
            "type": "example.import.validator",
            "schema_ref": "example.import.validator@v1",
            "schema_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
        });
        let mut policy = json!({
            "id": "epolicy-import-validator-v1",
            "schema_version": "evidence.contract.v1",
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "defaults": {
                "preset_id": "import-validator",
                "binding": true,
                "assurance_level": "standard"
            },
            "named_presets": [{
                "id": "import-validator",
                "schema_version": "evidence.contract.v1",
                "namespace": "example.import.validator",
                "observations": [{
                    "id": "validator-passed",
                    "type": "example.import.validator",
                    "subject": "generic import validator",
                    "expected": {"status": "passed"},
                    "target": {"kind": "process", "uri": "local://generic-validator"}
                }]
            }],
            "observation_schema_registrations": [{
                "type": "example.import.validator",
                "schema_ref": "example.import.validator@v1",
                "schema_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                "owning_namespace": "example.import.validator"
            }],
            "adapter_registrations": [{
                "manifest_id": "vcap-import-validator-v1",
                "manifest_path": ".planr/evidence/adapters/import-validator.manifest.json",
                "manifest_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "observation_types": ["example.import.validator"],
                "payload_schemas": [payload_schema.clone()],
                "provenance_path": "planr_observed_execution",
                "execution_contract": {
                    "kind": "process",
                    "executable": "node",
                    "args": ["generic-validator.mjs"],
                    "working_directory": ".",
                    "timeout_ms": 5000,
                    "stdout_limit_bytes": 4096,
                    "stderr_limit_bytes": 4096,
                    "payload_schema": payload_schema
                }
            }],
            "extension_namespaces": ["example.import.validator"],
            "trust_policy": {
                "accepted_provenance": ["planr_observed_execution"],
                "min_receipt_status": "trusted",
                "allow_user_attestation": false
            },
            "freshness_policy": {
                "max_age_seconds": 3600,
                "invalidate_on": ["source_change", "target_change", "policy_change"]
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
                    "policy_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }]
            }
        });
        let policy_digest = crate::canonical_json::sha256_json_digest_without_top_level_field(
            &policy,
            "policy_digest",
        )
        .unwrap();
        policy["policy_digest"] = json!(policy_digest);
        serde_yaml::to_string(&policy).unwrap()
    }

    #[test]
    fn generic_validator_stdout_accepts_optional_planr_schema_binding_only() {
        let result = json!({
            "kind": "planr.import.validator.generic_predicate.result",
            "version": "1.0.0",
            "verdict": "passed",
            "artifact_set_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "predicate_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "verifier_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "verifier_instance_digest": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        });
        let stdout = result.to_string();
        let stdout_digest = sha256_prefixed_bytes(stdout.as_bytes());
        let raw_result = json!({
            "kind": "process_output",
            "exit": {"exit_code": 0},
            "stdout_truncated": false,
            "stderr_truncated": false,
            "stdout_bytes": stdout.len(),
            "stderr_bytes": 0,
            "stdout_digest": stdout_digest,
            "stderr_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "stdout_excerpt": stdout,
        });
        let raw_result_digest = sha256_json_digest(&raw_result).unwrap();
        let actual = result.as_object().unwrap().clone();

        assert!(
            validator_stdout_result(&actual, &raw_result, &stdout_digest, &raw_result_digest,)
                .is_ok()
        );

        let mut schema_bound = actual.clone();
        schema_bound.insert(
            "schema_ref".to_string(),
            json!("schema://planr.import.validator.generic_predicate"),
        );
        assert!(
            validator_stdout_result(
                &schema_bound,
                &raw_result,
                &stdout_digest,
                &raw_result_digest,
            )
            .is_ok()
        );

        schema_bound.insert("schema_ref".to_string(), json!(false));
        assert!(
            validator_stdout_result(
                &schema_bound,
                &raw_result,
                &stdout_digest,
                &raw_result_digest,
            )
            .is_err()
        );
    }

    fn process_adapter_digest(root: &Path, execution: &ProcessExecutionContract) -> String {
        sha256_json_digest(&json!({
            "schema_version": "planr.process_adapter.binding.v1",
            "execution_contract": execution,
            "file_arguments": [process_adapter_file_argument_identity(root, execution, 0)],
        }))
        .unwrap()
    }

    fn process_adapter_file_argument_identity(
        root: &Path,
        execution: &ProcessExecutionContract,
        index: usize,
    ) -> Value {
        let argument = execution.args[index].as_str();
        let cwd = root
            .join(execution.working_directory.as_deref().unwrap_or("."))
            .canonicalize()
            .unwrap();
        let canonical = cwd.join(argument).canonicalize().unwrap();
        let relative = canonical
            .strip_prefix(&cwd)
            .unwrap()
            .to_string_lossy()
            .to_string();
        json!({
            "argument_index": index,
            "argument": argument,
            "resolved_relative_to": "command_cwd",
            "cwd": cwd.to_string_lossy(),
            "path": canonical.to_string_lossy(),
            "cwd_relative_path": relative,
            "path_digest": sha256_prefixed_bytes(canonical.to_string_lossy().as_bytes()),
            "content_digest": sha256_prefixed_bytes(&fs::read(&canonical).unwrap()),
        })
    }

    fn fixture_root() -> TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".planr")).unwrap();
        fs::write(
            temp.path().join(".planr/evidence.yaml"),
            repository_policy_yaml(),
        )
        .unwrap();
        fs::write(
            temp.path().join("health-output.json"),
            br#"{"status":"ok"}"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("test-results.xml"),
            br#"<testsuite name="unit" tests="1" failures="0" errors="0"><testcase name="passes" /></testsuite>"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("test-results-failing.xml"),
            br#"<testsuite name="unit" tests="1" failures="1" errors="0"><testcase name="fails"><failure /></testcase></testsuite>"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("extra-artifact.json"),
            br#"{"uninterpreted":true}"#,
        )
        .unwrap();
        fs::write(temp.path().join("runner.json"), br#"{"command":["cargo","test"],"exit_code":0,"status":"passed","stdout_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","stderr_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","duration_ms":12}"#).unwrap();
        fs::write(temp.path().join("runner-failing.json"), br#"{"command":["cargo","test"],"exit_code":1,"status":"failed","stdout_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","stderr_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","duration_ms":12}"#).unwrap();
        fs::write(
            temp.path().join("generic-validator.mjs"),
            br#"import crypto from "node:crypto";
import fs from "node:fs";

const sortJson = (value) => {
  if (Array.isArray(value)) return value.map(sortJson);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortJson(value[key])]));
  }
  return value;
};
const canonical = (value) => JSON.stringify(sortJson(value));
const digestBytes = (bytes) => `sha256:${crypto.createHash("sha256").update(bytes).digest("hex")}`;
const digestJson = (value) => digestBytes(Buffer.from(canonical(value)));

const artifact = fs.readFileSync(process.env.ARTIFACT_PATH);
const artifacts = process.env.ARTIFACTS_JSON
  ? JSON.parse(process.env.ARTIFACTS_JSON).map((entry) => ({
      id: entry.id,
      kind: entry.kind,
      declared_digest: entry.declared_digest,
      content_digest: digestBytes(fs.readFileSync(entry.path)),
    }))
  : [{
      id: process.env.ARTIFACT_ID,
      kind: process.env.ARTIFACT_KIND,
      declared_digest: process.env.ARTIFACT_DECLARED_DIGEST,
      content_digest: digestBytes(artifact),
    }];
const predicate = JSON.parse(process.env.PREDICATE_JSON);
const actual = JSON.parse(process.env.ACTUAL_JSON);
const artifactSetDigest = digestJson(artifacts);
const predicateDigest = digestJson({
  kind: "generic_versioned_adapter_predicate",
  version: "1.0.0",
  type: process.env.OBSERVATION_TYPE,
  outcome: process.env.OUTCOME,
  predicate,
  actual,
});
const result = {
  kind: "planr.import.validator.generic_predicate.result",
  version: "1.0.0",
  verdict: "passed",
  artifact_set_digest: artifactSetDigest,
  predicate_digest: predicateDigest,
  verifier_digest: process.env.VERIFIER_DIGEST,
  verifier_instance_digest: process.env.VERIFIER_INSTANCE_DIGEST,
};
switch (process.env.RESULT_MUTATION ?? "") {
  case "wrong-kind":
    result.kind = "planr.import.validator.other";
    break;
  case "missing-kind":
    delete result.kind;
    break;
  case "unsupported-version":
    result.version = "9.9.9";
    break;
  case "missing-version":
    delete result.version;
    break;
  case "unknown-field":
    result.untrusted_extra = true;
    break;
}
process.stdout.write(JSON.stringify(result));
"#,
        )
        .unwrap();
        fs::write(temp.path().join("silent-validator.mjs"), b"").unwrap();
        init_git_repo(temp.path());
        temp
    }

    fn init_git_repo(root: &Path) {
        git(root, &["init"]);
        git(
            root,
            &["config", "user.email", "planr-test@example.invalid"],
        );
        git(root, &["config", "user.name", "Planr Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial evidence import fixture"]);
    }

    fn git(root: &Path, args: &[&str]) {
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

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        crate::storage::ensure_schema(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-evidence', 'Evidence', '.', 'active', datetime('now'), datetime('now'));
             INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
             VALUES ('pln-evidence', 'p-evidence', 'build', '/tmp/planr/import.plan.md', 'Evidence Import Plan', 'evidence-import-plan', 'ok', 'hash-evidence-import', datetime('now'), datetime('now'))",
        )
        .unwrap();
        conn
    }

    fn seed_verifier(conn: &Connection, formats: &[&str], observations: &[&str]) {
        seed_verifier_with_contract(
            conn,
            formats,
            observations,
            generic_validator_execution_contract("generic-validator.mjs"),
        );
    }

    fn seed_verifier_with_root(
        conn: &Connection,
        root: &Path,
        formats: &[&str],
        observations: &[&str],
    ) {
        seed_verifier_with_contract_and_digest(
            conn,
            formats,
            observations,
            generic_validator_execution_contract("generic-validator.mjs"),
            |execution_contract| process_adapter_digest(root, execution_contract),
        );
    }

    fn seed_generic_verifier_with_root(conn: &Connection, root: &Path) {
        seed_verifier_with_root(
            conn,
            root,
            &[GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT],
            &[
                "planr.test.health",
                "planr.import.validator.generic_predicate",
            ],
        );
    }

    fn seed_verifier_with_contract(
        conn: &Connection,
        formats: &[&str],
        observations: &[&str],
        execution_contract: ProcessExecutionContract,
    ) {
        seed_verifier_with_contract_and_digest(
            conn,
            formats,
            observations,
            execution_contract,
            |_| {
                "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string()
            },
        );
    }

    fn seed_verifier_with_contract_at_root(
        conn: &Connection,
        root: &Path,
        formats: &[&str],
        observations: &[&str],
        execution_contract: ProcessExecutionContract,
    ) {
        seed_verifier_with_contract_and_digest(
            conn,
            formats,
            observations,
            execution_contract,
            |execution_contract| process_adapter_digest(root, execution_contract),
        );
    }

    fn seed_verifier_with_contract_and_digest(
        conn: &Connection,
        formats: &[&str],
        observations: &[&str],
        execution_contract: ProcessExecutionContract,
        adapter_digest: impl FnOnce(&ProcessExecutionContract) -> String,
    ) {
        let manifest = manifest_value(formats, observations);
        let mut manifest = manifest;
        manifest["availability_probe"]["execution"] = json!(execution_contract);
        manifest["adapter_digest"] = json!(adapter_digest(&execution_contract));
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        let instance = instance_value(&manifest_digest, observations[0]);
        insert_verifier_rows(
            conn,
            &manifest,
            &manifest_digest,
            &instance,
            &execution_contract,
        );
    }

    fn seed_verifier_with_manifest_digest(
        conn: &Connection,
        formats: &[&str],
        observations: &[&str],
        manifest_digest: &str,
    ) {
        let manifest = manifest_value(formats, observations);
        let instance = instance_value(manifest_digest, observations[0]);
        insert_verifier_rows(
            conn,
            &manifest,
            manifest_digest,
            &instance,
            &generic_validator_execution_contract("generic-validator.mjs"),
        );
    }

    fn seed_verifier_with_instance(
        conn: &Connection,
        formats: &[&str],
        observations: &[&str],
        instance: Value,
    ) {
        let manifest = manifest_value(formats, observations);
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        insert_verifier_rows(
            conn,
            &manifest,
            &manifest_digest,
            &instance,
            &generic_validator_execution_contract("generic-validator.mjs"),
        );
    }

    fn insert_verifier_rows(
        conn: &Connection,
        manifest: &Value,
        manifest_digest: &str,
        instance: &Value,
        execution_contract: &ProcessExecutionContract,
    ) {
        let probe_result = instance["probe_result"].clone();
        let execution_contract_digest =
            sha256_json_digest(&serde_json::to_value(execution_contract).unwrap()).unwrap();
        let host_fingerprint = json!({
            "environment": instance["environment"].clone(),
            "execution_contract_digest": execution_contract_digest,
            "execution_contract_source": "test_registry",
        });
        conn.execute(
            "INSERT INTO verification_capability_manifests(
              id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, created_at
            ) VALUES (?1, '1.0.0', 'artifact_import', ?2, ?3, ?4, datetime('now'))",
            params![
                "verifier-generic-adapter",
                manifest["adapter_digest"].as_str().unwrap(),
                manifest_digest,
                manifest.to_string(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verification_capability_instances(
              id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
              availability_status, runtime_target_json, host_fingerprint_json,
              capability_snapshot_json, probe_result_json, created_at
            ) VALUES (
              'instance-generic-adapter', 'verifier-generic-adapter', '1.0.0',
              ?1, 'probe-1', 'available', ?2, ?3, ?4, ?5, datetime('now')
            )",
            params![
                manifest_digest,
                manifest["runtime_targets"].to_string(),
                host_fingerprint.to_string(),
                instance.to_string(),
                probe_result.to_string(),
            ],
        )
        .unwrap();
    }

    fn insert_verifier_instance_row(
        conn: &Connection,
        manifest: &Value,
        manifest_digest: &str,
        instance: &Value,
        execution_contract: &ProcessExecutionContract,
    ) {
        let probe_result = instance["probe_result"].clone();
        let execution_contract_digest =
            sha256_json_digest(&serde_json::to_value(execution_contract).unwrap()).unwrap();
        let host_fingerprint = json!({
            "environment": instance["environment"].clone(),
            "execution_contract_digest": execution_contract_digest,
            "execution_contract_source": "test_registry",
        });
        conn.execute(
            "INSERT INTO verification_capability_instances(
              id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
              availability_status, runtime_target_json, host_fingerprint_json,
              capability_snapshot_json, probe_result_json, created_at
            ) VALUES (
              ?1, 'verifier-generic-adapter', '1.0.0',
              ?2, ?3, 'available', ?4, ?5, ?6, ?7, datetime('now', '+1 second')
            )",
            params![
                instance["id"].as_str().unwrap(),
                manifest_digest,
                instance["probe_result"]["probe_execution_id"]
                    .as_str()
                    .unwrap(),
                manifest["runtime_targets"].to_string(),
                host_fingerprint.to_string(),
                instance.to_string(),
                probe_result.to_string(),
            ],
        )
        .unwrap();
    }

    fn registered_manifest(conn: &Connection) -> (Value, String, ProcessExecutionContract) {
        let (manifest_json, manifest_digest): (String, String) = conn
            .query_row(
                "SELECT manifest_json, manifest_digest
                 FROM verification_capability_manifests
                 WHERE id = 'verifier-generic-adapter' AND version = '1.0.0'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let manifest: Value = serde_json::from_str(&manifest_json).unwrap();
        let execution_contract =
            serde_json::from_value(manifest["availability_probe"]["execution"].clone()).unwrap();
        (manifest, manifest_digest, execution_contract)
    }

    fn manifest_value(formats: &[&str], observations: &[&str]) -> Value {
        let supported_observations = observations
            .iter()
            .map(|observation| {
                json!({
                    "type": observation,
                    "schema_ref": if *observation == "planr.import.validator.generic_predicate" {
                        "schema://planr.test.health".to_string()
                    } else {
                        format!("schema://{observation}")
                    },
                    "schema_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
                })
            })
            .collect::<Vec<_>>();
        json!({
            "id": "verifier-generic-adapter",
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "artifact_import",
            "adapter_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "supported_surfaces": ["local-process"],
            "supported_observations": supported_observations,
            "supported_interactions": ["import"],
            "supported_artifacts": formats,
            "runtime_targets": [{"kind": "process", "id": "runtime-local"}],
            "provenance_path": "validated_artifact_import",
            "permissions": {},
            "costs": {},
            "determinism": "deterministic",
            "repeatability": "repeatable",
            "independence": "independent",
            "blind_spots": ["none"],
            "availability_probe": {
                "kind": "process",
                "execution": {
                    "kind": "process",
                    "executable": "true",
                    "args": [],
                    "timeout_ms": 1000,
                    "stdout_limit_bytes": 1,
                    "stderr_limit_bytes": 1,
                    "payload_schema": {
                        "type": observations[0],
                        "schema_ref": format!("schema://{}", observations[0]),
                        "schema_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                    }
                }
            }
        })
    }

    fn instance_value(manifest_digest: &str, observation: &str) -> Value {
        json!({
            "id": "instance-generic-adapter",
            "schema_version": "evidence.contract.v1",
            "manifest_id": "verifier-generic-adapter",
            "manifest_digest": manifest_digest,
            "host": "codex",
            "surface": "local-process",
            "host_version": "test",
            "adapter_version": "1.0.0",
            "environment": {"kind": "local", "id": "env-local", "digest": "sha256:5555555555555555555555555555555555555555555555555555555555555555"},
            "permissions": {"network": "none", "filesystem": "read"},
            "availability": {"status": "available"},
            "probe_result": {"probe_execution_id": "probe-1", "outcome": "passed", "observed_at": "2026-07-28T12:00:00Z", "checks": [{"name": "validator-available", "outcome": "passed"}]},
            "observed_payload_contract": {"schema_ref": format!("schema://{observation}"), "observation_types": [observation, "planr.import.validator.generic_predicate"]},
            "limitations": ["none"],
            "captured_at": "2026-07-28T12:00:00Z"
        })
    }

    fn unrelated_newer_instance_value(manifest_digest: &str, observation: &str) -> Value {
        let mut instance = instance_value(manifest_digest, observation);
        instance["id"] = json!("instance-unrelated-newer");
        instance["environment"]["id"] = json!("env-unrelated");
        instance["environment"]["digest"] =
            json!("sha256:6666666666666666666666666666666666666666666666666666666666666666");
        instance["probe_result"]["probe_execution_id"] = json!("probe-unrelated");
        instance["captured_at"] = json!("2026-07-28T12:00:01Z");
        instance
    }

    fn repository<'a>(
        conn: &'a Connection,
        root: &'a TempDir,
    ) -> ValidatedArtifactImportRepository<'a> {
        ValidatedArtifactImportRepository {
            conn,
            project_id: "p-evidence",
            artifact_root: root.path(),
        }
    }

    fn repository_for_path<'a>(
        conn: &'a Connection,
        root: &'a Path,
    ) -> ValidatedArtifactImportRepository<'a> {
        ValidatedArtifactImportRepository {
            conn,
            project_id: "p-evidence",
            artifact_root: root,
        }
    }

    struct GenericAttestationInput {
        artifacts: Vec<GenericAttestationArtifactInput>,
        observation_type: &'static str,
        outcome: &'static str,
        predicate: Value,
        actual: Value,
    }

    struct GenericAttestationArtifactInput {
        artifact_id: &'static str,
        artifact_kind: &'static str,
        declared_digest: String,
        content: &'static str,
    }

    fn generic_validator_execution_contract(script_name: &str) -> ProcessExecutionContract {
        serde_json::from_value(json!({
            "kind": "process",
            "executable": "node",
            "args": [script_name],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 1024,
            "payload_schema": {
                "type": "planr.import.validator.generic_predicate",
                "schema_ref": "schema://planr.test.health",
                "schema_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
            }
        }))
        .unwrap()
    }

    fn valid_import() -> Value {
        let attestation = generic_attestation(GenericAttestationInput {
            artifacts: vec![GenericAttestationArtifactInput {
                artifact_id: "artifact-health-output",
                artifact_kind: "stdout-json",
                declared_digest: digest(r#"{"status":"ok"}"#),
                content: r#"{"status":"ok"}"#,
            }],
            observation_type: "planr.test.health",
            outcome: "passed",
            predicate: json!({"expected": "ok"}),
            actual: json!({"status": "ok"}),
        });
        json!({
            "id": "import-health-check",
            "schema_version": VALIDATED_ARTIFACT_IMPORT_SCHEMA_VERSION,
            "source_kind": "artifact_import",
            "submitted_at": "2026-07-28T12:00:00Z",
            "format": GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT,
            "verifier_identity": {"kind": "adapter", "id": "verifier-generic-adapter", "name": "verifier-generic-adapter", "version": "1.0.0", "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"},
            "adapter_predicate": {"kind": "generic_versioned_adapter_predicate", "version": "1.0.0", "type": "planr.test.health", "outcome": "passed", "predicate": {"expected": "ok"}, "actual": {"status": "ok"}, "attestation": attestation},
            "artifact_refs": [{"id": "artifact-health-output", "kind": "stdout-json", "digest": digest(r#"{"status":"ok"}"#), "uri": "file://health-output.json"}],
            "producer_metadata": {"client": "fixture-importer"}
        })
    }

    fn valid_multi_artifact_import() -> Value {
        let mut import = valid_import();
        import["id"] = json!("import-health-check-multi-artifact");
        import["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "artifact-extra", "kind": "stdout-json", "digest": digest(r#"{"uninterpreted":true}"#), "uri": "file://extra-artifact.json"}));
        import["adapter_predicate"]["attestation"] = generic_attestation(GenericAttestationInput {
            artifacts: vec![
                GenericAttestationArtifactInput {
                    artifact_id: "artifact-health-output",
                    artifact_kind: "stdout-json",
                    declared_digest: digest(r#"{"status":"ok"}"#),
                    content: r#"{"status":"ok"}"#,
                },
                GenericAttestationArtifactInput {
                    artifact_id: "artifact-extra",
                    artifact_kind: "stdout-json",
                    declared_digest: digest(r#"{"uninterpreted":true}"#),
                    content: r#"{"uninterpreted":true}"#,
                },
            ],
            observation_type: "planr.test.health",
            outcome: "passed",
            predicate: json!({"expected": "ok"}),
            actual: json!({"status": "ok"}),
        });
        import
    }

    fn valid_runner_import(id: &str) -> Value {
        json!({
            "id": id,
            "schema_version": VALIDATED_ARTIFACT_IMPORT_SCHEMA_VERSION,
            "source_kind": "artifact_import",
            "submitted_at": "2026-07-28T12:00:00Z",
            "format": PLANR_RUNNER_RESULT_FORMAT,
            "verifier_identity": {"kind": "adapter", "id": "verifier-generic-adapter", "name": "verifier-generic-adapter", "version": "1.0.0", "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"},
            "runner_result": {"kind": "planr_runner_result", "version": "1.0.0", "command": ["cargo", "test"], "exit_code": 0, "status": "passed", "stdout_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "stderr_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "duration_ms": 12},
            "artifact_refs": [{"id": "artifact-runner", "kind": "runner-json", "digest": digest(r#"{"command":["cargo","test"],"exit_code":0,"status":"passed","stdout_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","stderr_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","duration_ms":12}"#), "uri": "file://runner.json"}],
            "producer_metadata": {"client": "fixture-importer"}
        })
    }

    fn generic_attestation(input: GenericAttestationInput) -> Value {
        let artifacts = input
            .artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "id": artifact.artifact_id,
                    "kind": artifact.artifact_kind,
                    "declared_digest": artifact.declared_digest,
                    "content_digest": sha256_prefixed_bytes(artifact.content.as_bytes()),
                })
            })
            .collect::<Vec<_>>();
        let artifact_set_digest = sha256_json_digest(&Value::Array(artifacts)).unwrap();
        let predicate_digest = sha256_json_digest(&json!({"kind": "generic_versioned_adapter_predicate", "version": "1.0.0", "type": input.observation_type, "outcome": input.outcome, "predicate": input.predicate, "actual": input.actual})).unwrap();
        let verifier_digest =
            "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let manifest = manifest_value(
            &[GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT],
            &[input.observation_type],
        );
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        let instance = instance_value(&manifest_digest, input.observation_type);
        let verifier_instance_digest = sha256_json_digest(&instance).unwrap();
        let probe_result_digest = sha256_json_digest(&instance["probe_result"]).unwrap();
        json!({"kind": "planr_import_validator_attestation", "version": "1.0.0", "artifact_set_digest": artifact_set_digest, "predicate_digest": predicate_digest, "verifier_digest": verifier_digest, "verifier_instance_digest": verifier_instance_digest, "probe_execution_id": "probe-1", "probe_result_digest": probe_result_digest, "validator_attempt_id": "pending-validator-attempt", "validator_receipt_id": "pending-validator-receipt", "validator_receipt_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000"})
    }

    fn seed_generic_validator_observation(conn: &Connection, root: &Path, import: &mut Value) {
        seed_generic_validator_observation_with_script(conn, root, import, "generic-validator.mjs");
    }

    fn seed_generic_validator_observation_with_script(
        conn: &Connection,
        root: &Path,
        import: &mut Value,
        script_name: &str,
    ) {
        seed_generic_validator_observation_with_options(conn, root, import, script_name, None);
    }

    fn seed_generic_validator_observation_with_result_mutation(
        conn: &Connection,
        root: &Path,
        import: &mut Value,
        result_mutation: &str,
    ) {
        seed_generic_validator_observation_with_options(
            conn,
            root,
            import,
            "generic-validator.mjs",
            Some(result_mutation),
        );
    }

    fn seed_generic_validator_observation_with_options(
        conn: &Connection,
        root: &Path,
        import: &mut Value,
        script_name: &str,
        result_mutation: Option<&str>,
    ) {
        let execution_contract = generic_validator_execution_contract(script_name);
        let verifier_digest: String = conn
            .query_row(
                "SELECT adapter_digest FROM verification_capability_manifests
                 WHERE id = 'verifier-generic-adapter' AND version = '1.0.0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        import["verifier_identity"]["digest"] = json!(verifier_digest);
        import["adapter_predicate"]["attestation"]["verifier_digest"] = json!(verifier_digest);
        let snapshot: String = conn
            .query_row(
                "SELECT capability_snapshot_json FROM verification_capability_instances WHERE id = 'instance-generic-adapter'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let snapshot_value: Value = serde_json::from_str(&snapshot).unwrap();
        let verifier_instance_digest = sha256_json_digest(&snapshot_value).unwrap();
        import["adapter_predicate"]["attestation"]["verifier_instance_digest"] =
            json!(verifier_instance_digest);
        let attestation = &import["adapter_predicate"]["attestation"];
        let expected = json!({
            "artifact_set_digest": attestation["artifact_set_digest"],
            "predicate_digest": attestation["predicate_digest"],
            "verifier_digest": attestation["verifier_digest"],
            "verifier_instance_digest": attestation["verifier_instance_digest"],
        });
        let target = json!({"kind": "process", "uri": "local://generic-validator"});
        let environment = json!({"kind": "local", "id": "env-local", "digest": "sha256:5555555555555555555555555555555555555555555555555555555555555555"});
        let observations = json!([{
            "id": "obs-generic-validator",
            "type": "planr.import.validator.generic_predicate",
            "subject": "validated-artifact-import",
            "expected": expected,
            "target": target.clone(),
            "payload_schema": {"schema_ref": "schema://planr.test.health"}
        }]);
        let obligation: ProofObligation = serde_json::from_value(json!({
            "id": "obl-generic-validator",
            "schema_version": "evidence.contract.v1",
            "criterion_id": "crit-generic-validator",
            "plan_id": "pln-evidence",
            "title": "Generic validator observation",
            "binding": true,
            "observations": observations,
            "fixture_policy": {"fixtures_allowed": false, "mocks_allowed": false, "disclosure_required": false},
            "freshness_policy": {},
            "assurance_policy": {}
        }))
        .unwrap();
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, created_at
            ) VALUES (
              ?1, 'p-evidence', ?2, NULL, ?3, 1, ?4, 1, ?5, ?6, '{}', '{}', ?7, ?8, ?9
            )",
            params![
                obligation.id.as_str(),
                obligation.plan_id.as_str(),
                obligation.criterion_id.as_str(),
                obligation.title.as_str(),
                serde_json::to_string(&obligation.observations).unwrap(),
                serde_json::to_string(&obligation.fixture_policy).unwrap(),
                "sha256:7777777777777777777777777777777777777777777777777777777777777777",
                "sha256:8888888888888888888888888888888888888888888888888888888888888888",
                "2026-07-29T00:00:00Z",
            ],
        )
        .unwrap();
        let snapshot: String = conn
            .query_row(
                "SELECT capability_snapshot_json FROM verification_capability_instances WHERE id = 'instance-generic-adapter'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let instance: VerificationCapabilityInstance = serde_json::from_str(&snapshot).unwrap();
        let cancellation = CancellationToken::new();
        let artifact = &import["artifact_refs"][0];
        let mut env = BTreeMap::new();
        env.insert(
            "ARTIFACT_PATH".to_string(),
            root.join(
                artifact["uri"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("file://")
                    .unwrap(),
            )
            .to_string_lossy()
            .to_string(),
        );
        env.insert(
            "ARTIFACT_ID".to_string(),
            artifact["id"].as_str().unwrap().to_string(),
        );
        env.insert(
            "ARTIFACT_KIND".to_string(),
            artifact["kind"].as_str().unwrap().to_string(),
        );
        env.insert(
            "ARTIFACT_DECLARED_DIGEST".to_string(),
            artifact["digest"].as_str().unwrap().to_string(),
        );
        let artifacts = import["artifact_refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|artifact| {
                json!({
                    "id": artifact["id"],
                    "kind": artifact["kind"],
                    "declared_digest": artifact["digest"],
                    "path": root.join(
                        artifact["uri"]
                            .as_str()
                            .unwrap()
                            .strip_prefix("file://")
                            .unwrap(),
                    )
                    .to_string_lossy()
                    .to_string(),
                })
            })
            .collect::<Vec<_>>();
        env.insert(
            "ARTIFACTS_JSON".to_string(),
            Value::Array(artifacts).to_string(),
        );
        env.insert(
            "OBSERVATION_TYPE".to_string(),
            import["adapter_predicate"]["type"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        env.insert(
            "OUTCOME".to_string(),
            import["adapter_predicate"]["outcome"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        env.insert(
            "PREDICATE_JSON".to_string(),
            import["adapter_predicate"]["predicate"].to_string(),
        );
        env.insert(
            "ACTUAL_JSON".to_string(),
            import["adapter_predicate"]["actual"].to_string(),
        );
        env.insert(
            "VERIFIER_DIGEST".to_string(),
            attestation["verifier_digest"].as_str().unwrap().to_string(),
        );
        env.insert(
            "VERIFIER_INSTANCE_DIGEST".to_string(),
            attestation["verifier_instance_digest"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        if let Some(result_mutation) = result_mutation {
            env.insert("RESULT_MUTATION".to_string(), result_mutation.to_string());
        }
        let output = run_configured_process_adapter(
            conn,
            ConfiguredProcessRunInput {
                repository_root: root,
                project_id: "p-evidence",
                obligation,
                capability_instance: instance,
                execution_contract,
                payload_json_schema: None,
                observation_payload_json_schemas: BTreeMap::new(),
                target: TargetBinding {
                    kind: "process".to_string(),
                    uri: Some("local://generic-validator".to_string()),
                    digest: None,
                    deployment_id: None,
                },
                environment: serde_json::from_value(environment).unwrap(),
                fixture_disclosure: FixtureDisclosure {
                    fixtures_used: false,
                    mocks_used: false,
                    fixture_refs: None,
                    mock_refs: None,
                },
                env,
                retry_of: None,
                attempt_index: 0,
                max_attempts: 1,
                execution_binding: json!({
                    "schema_version": "planr.evidence.execution-binding.v2",
                    "run_index_digest": "sha256:9999999999999999999999999999999999999999999999999999999999999999",
                    "run_index": 0,
                    "obligation_id": "obl-generic-validator",
                    "target": {"kind": "process", "uri": "local://generic-validator"},
                    "requirement_ids": ["obs-generic-validator"]
                }),
                cancellation: &cancellation,
            },
        )
        .unwrap();
        import["adapter_predicate"]["attestation"]["validator_attempt_id"] =
            json!(output.attempt.id.as_str());
        import["adapter_predicate"]["attestation"]["validator_receipt_id"] =
            json!(output.receipt_value["id"].as_str().unwrap());
        import["adapter_predicate"]["attestation"]["validator_receipt_digest"] =
            json!(output.receipt_digest);
    }

    fn point_attestation_at_receipt_with_mismatched_embedded_obligation(
        conn: &Connection,
        import: &mut Value,
    ) {
        let original_receipt_id =
            import["adapter_predicate"]["attestation"]["validator_receipt_id"]
                .as_str()
                .unwrap();
        let (
            project_id,
            row_obligation_id,
            attempt_id,
            trusted_binding_json,
            observations_json,
            provenance_json,
            receipt_json,
        ): (String, String, String, String, String, String, String) = conn
            .query_row(
                "SELECT project_id, obligation_id, attempt_id, trusted_binding_json,
                        observations_json, provenance_json, receipt_json
                 FROM evidence_receipts
                 WHERE id = ?1",
                [original_receipt_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        let mut receipt_value: Value = serde_json::from_str(&receipt_json).unwrap();
        receipt_value["obligation_id"] = json!("obl-forged-validator");
        receipt_value["receipt_digest"] = Value::Null;
        let new_digest = {
            let mut digest_value = receipt_value.clone();
            digest_value
                .as_object_mut()
                .unwrap()
                .remove("receipt_digest");
            sha256_json_digest(&digest_value).unwrap()
        };
        receipt_value["receipt_digest"] = json!(new_digest);
        let new_receipt_id = format!("{original_receipt_id}-forged-obligation");
        conn.execute(
            "INSERT INTO evidence_receipts(
              id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
              trusted_binding_json, observations_json, provenance_json, receipt_json,
              supersedes_receipt_id, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, 'trusted', ?5, ?6, ?7, ?8, ?9, NULL, datetime('now')
            )",
            params![
                new_receipt_id,
                project_id,
                row_obligation_id,
                attempt_id,
                new_digest,
                trusted_binding_json,
                observations_json,
                provenance_json,
                receipt_value.to_string(),
            ],
        )
        .unwrap();
        import["adapter_predicate"]["attestation"]["validator_receipt_id"] = json!(new_receipt_id);
        import["adapter_predicate"]["attestation"]["validator_receipt_digest"] = json!(new_digest);
    }

    fn point_attestation_at_receipt_with_forged_stdout(conn: &Connection, import: &mut Value) {
        let original_receipt_id =
            import["adapter_predicate"]["attestation"]["validator_receipt_id"]
                .as_str()
                .unwrap();
        let (
            project_id,
            row_obligation_id,
            attempt_id,
            trusted_binding_json,
            observations_json,
            provenance_json,
            receipt_json,
        ): (String, String, String, String, String, String, String) = conn
            .query_row(
                "SELECT project_id, obligation_id, attempt_id, trusted_binding_json,
                        observations_json, provenance_json, receipt_json
                 FROM evidence_receipts
                 WHERE id = ?1",
                [original_receipt_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        let mut receipt_value: Value = serde_json::from_str(&receipt_json).unwrap();
        let attestation = &import["adapter_predicate"]["attestation"];
        receipt_value["observations"][0]["actual"]["stdout_excerpt"] = json!(
            json!({
                "kind": "planr.import.validator.generic_predicate.result",
                "version": "1.0.0",
                "verdict": "passed",
                "artifact_set_digest": attestation["artifact_set_digest"],
                "predicate_digest": attestation["predicate_digest"],
                "verifier_digest": attestation["verifier_digest"],
                "verifier_instance_digest": attestation["verifier_instance_digest"],
                "forged_after_attempt": true,
            })
            .to_string()
        );
        receipt_value["receipt_digest"] = Value::Null;
        let new_digest = {
            let mut digest_value = receipt_value.clone();
            digest_value
                .as_object_mut()
                .unwrap()
                .remove("receipt_digest");
            sha256_json_digest(&digest_value).unwrap()
        };
        receipt_value["receipt_digest"] = json!(new_digest);
        let new_receipt_id = format!("{original_receipt_id}-forged-stdout");
        conn.execute(
            "INSERT INTO evidence_receipts(
              id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
              trusted_binding_json, observations_json, provenance_json, receipt_json,
              supersedes_receipt_id, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, 'trusted', ?5, ?6, ?7, ?8, ?9, NULL, datetime('now')
            )",
            params![
                new_receipt_id,
                project_id,
                row_obligation_id,
                attempt_id,
                new_digest,
                trusted_binding_json,
                observations_json,
                provenance_json,
                receipt_value.to_string(),
            ],
        )
        .unwrap();
        import["adapter_predicate"]["attestation"]["validator_receipt_id"] = json!(new_receipt_id);
        import["adapter_predicate"]["attestation"]["validator_receipt_digest"] = json!(new_digest);
    }

    fn point_attestation_at_receipt_with_forged_raw_result_metadata(
        conn: &Connection,
        import: &mut Value,
    ) {
        let original_receipt_id =
            import["adapter_predicate"]["attestation"]["validator_receipt_id"]
                .as_str()
                .unwrap();
        let (
            project_id,
            row_obligation_id,
            attempt_id,
            trusted_binding_json,
            observations_json,
            provenance_json,
            receipt_json,
        ): (String, String, String, String, String, String, String) = conn
            .query_row(
                "SELECT project_id, obligation_id, attempt_id, trusted_binding_json,
                        observations_json, provenance_json, receipt_json
                 FROM evidence_receipts
                 WHERE id = ?1",
                [original_receipt_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        let mut receipt_value: Value = serde_json::from_str(&receipt_json).unwrap();
        receipt_value["observations"][0]["actual"]["cwd"] =
            json!("/tmp/forged-validator-working-directory");
        let forged_raw_result_digest =
            sha256_json_digest(&receipt_value["observations"][0]["actual"]).unwrap();
        receipt_value["raw_result"]["digest"] = json!(forged_raw_result_digest);
        receipt_value["receipt_digest"] = Value::Null;
        let new_digest = {
            let mut digest_value = receipt_value.clone();
            digest_value
                .as_object_mut()
                .unwrap()
                .remove("receipt_digest");
            sha256_json_digest(&digest_value).unwrap()
        };
        receipt_value["receipt_digest"] = json!(new_digest);
        let new_receipt_id = format!("{original_receipt_id}-forged-raw-result");
        conn.execute(
            "INSERT INTO evidence_receipts(
              id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
              trusted_binding_json, observations_json, provenance_json, receipt_json,
              supersedes_receipt_id, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, 'trusted', ?5, ?6, ?7, ?8, ?9, NULL, datetime('now')
            )",
            params![
                new_receipt_id,
                project_id,
                row_obligation_id,
                attempt_id,
                new_digest,
                trusted_binding_json,
                observations_json,
                provenance_json,
                receipt_value.to_string(),
            ],
        )
        .unwrap();
        import["adapter_predicate"]["attestation"]["validator_receipt_id"] = json!(new_receipt_id);
        import["adapter_predicate"]["attestation"]["validator_receipt_digest"] = json!(new_digest);
    }

    #[test]
    fn validated_import_uses_typed_persisted_registry_binding_and_returns_record() {
        let root = fixture_root();
        let conn = conn();
        seed_generic_verifier_with_root(&conn, root.path());
        let mut import = valid_import();
        seed_generic_validator_observation(&conn, root.path(), &mut import);
        let (manifest, manifest_digest, execution_contract) = registered_manifest(&conn);
        insert_verifier_instance_row(
            &conn,
            &manifest,
            &manifest_digest,
            &unrelated_newer_instance_value(&manifest_digest, "planr.test.health"),
            &execution_contract,
        );
        let record = parse_validated_artifact_import(import, &repository(&conn, &root)).unwrap();

        assert!(!record.idempotent);
        assert_eq!(record.proposal.source_kind, "artifact_import");
        assert_eq!(
            record.proposal.producer_metadata["registered_verifier"]["manifest"]["id"],
            "verifier-generic-adapter"
        );
        assert_eq!(
            record.proposal.producer_metadata["registered_verifier"]["kind"],
            "adapter"
        );
        assert_eq!(
            record.proposal.producer_metadata["registered_verifier"]["name"],
            "verifier-generic-adapter"
        );
        assert!(
            record.proposal.producer_metadata["registered_verifier"]["instance"]["digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert_eq!(
            record.proposal.producer_metadata["registered_verifier"]["instance"]["id"],
            "instance-generic-adapter"
        );
    }

    #[test]
    fn generic_validator_requires_process_actual_and_matching_receipt_obligation() {
        let silent_root = fixture_root();
        let silent_conn = conn();
        seed_verifier_with_contract_at_root(
            &silent_conn,
            silent_root.path(),
            &[GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT],
            &[
                "planr.test.health",
                "planr.import.validator.generic_predicate",
            ],
            generic_validator_execution_contract("silent-validator.mjs"),
        );
        let mut silent_import = valid_import();
        seed_generic_validator_observation_with_script(
            &silent_conn,
            silent_root.path(),
            &mut silent_import,
            "silent-validator.mjs",
        );
        assert!(
            parse_validated_artifact_import(silent_import, &repository(&silent_conn, &silent_root))
                .is_err()
        );

        let copied_root = fixture_root();
        let copied_conn = conn();
        seed_generic_verifier_with_root(&copied_conn, copied_root.path());
        let mut copied_import = valid_import();
        copied_import["adapter_predicate"]["attestation"]["artifact_set_digest"] =
            json!("sha256:9999999999999999999999999999999999999999999999999999999999999999");
        copied_import["adapter_predicate"]["attestation"]["predicate_digest"] =
            json!("sha256:8888888888888888888888888888888888888888888888888888888888888888");
        seed_generic_validator_observation(&copied_conn, copied_root.path(), &mut copied_import);
        assert!(
            parse_validated_artifact_import(copied_import, &repository(&copied_conn, &copied_root))
                .is_err()
        );

        let stale_set_root = fixture_root();
        let stale_set_conn = conn();
        seed_generic_verifier_with_root(&stale_set_conn, stale_set_root.path());
        let mut stale_set_import = valid_import();
        stale_set_import["id"] = json!("import-health-check-stale-artifact-set");
        seed_generic_validator_observation(
            &stale_set_conn,
            stale_set_root.path(),
            &mut stale_set_import,
        );
        stale_set_import["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "artifact-extra", "kind": "stdout-json", "digest": digest(r#"{"uninterpreted":true}"#), "uri": "file://extra-artifact.json"}));
        assert!(
            parse_validated_artifact_import(
                stale_set_import,
                &repository(&stale_set_conn, &stale_set_root)
            )
            .is_err()
        );

        let multi_root = fixture_root();
        let multi_conn = conn();
        seed_generic_verifier_with_root(&multi_conn, multi_root.path());
        let mut multi_import = valid_multi_artifact_import();
        seed_generic_validator_observation(&multi_conn, multi_root.path(), &mut multi_import);
        let multi_record =
            parse_validated_artifact_import(multi_import, &repository(&multi_conn, &multi_root))
                .unwrap();
        assert_eq!(
            multi_record.proposal.claims["artifact_digests"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let obligation_root = fixture_root();
        let obligation_conn = conn();
        seed_generic_verifier_with_root(&obligation_conn, obligation_root.path());
        let mut obligation_import = valid_import();
        seed_generic_validator_observation(
            &obligation_conn,
            obligation_root.path(),
            &mut obligation_import,
        );
        point_attestation_at_receipt_with_mismatched_embedded_obligation(
            &obligation_conn,
            &mut obligation_import,
        );
        assert!(
            parse_validated_artifact_import(
                obligation_import,
                &repository(&obligation_conn, &obligation_root)
            )
            .is_err()
        );

        let stdout_root = fixture_root();
        let stdout_conn = conn();
        seed_generic_verifier_with_root(&stdout_conn, stdout_root.path());
        let mut stdout_import = valid_import();
        seed_generic_validator_observation(&stdout_conn, stdout_root.path(), &mut stdout_import);
        point_attestation_at_receipt_with_forged_stdout(&stdout_conn, &mut stdout_import);
        assert!(
            parse_validated_artifact_import(stdout_import, &repository(&stdout_conn, &stdout_root))
                .is_err()
        );

        let raw_result_root = fixture_root();
        let raw_result_conn = conn();
        seed_generic_verifier_with_root(&raw_result_conn, raw_result_root.path());
        let mut raw_result_import = valid_import();
        seed_generic_validator_observation(
            &raw_result_conn,
            raw_result_root.path(),
            &mut raw_result_import,
        );
        point_attestation_at_receipt_with_forged_raw_result_metadata(
            &raw_result_conn,
            &mut raw_result_import,
        );
        assert!(
            parse_validated_artifact_import(
                raw_result_import,
                &repository(&raw_result_conn, &raw_result_root)
            )
            .is_err()
        );

        for result_mutation in [
            "wrong-kind",
            "missing-kind",
            "unsupported-version",
            "missing-version",
            "unknown-field",
        ] {
            let root = fixture_root();
            let conn = conn();
            seed_generic_verifier_with_root(&conn, root.path());
            let mut import = valid_import();
            import["id"] = json!(format!("import-health-check-{result_mutation}"));
            seed_generic_validator_observation_with_result_mutation(
                &conn,
                root.path(),
                &mut import,
                result_mutation,
            );
            assert!(
                parse_validated_artifact_import(import, &repository(&conn, &root)).is_err(),
                "accepted mutated validator result {result_mutation}"
            );
        }
    }

    #[test]
    fn validated_import_rejects_invalid_snapshots_digest_mismatch_and_forged_predicate() {
        let root = fixture_root();
        let conn_digest_mismatch = conn();
        seed_verifier_with_manifest_digest(
            &conn_digest_mismatch,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
            "sha256:9999999999999999999999999999999999999999999999999999999999999999",
        );
        assert!(
            parse_validated_artifact_import(
                valid_runner_import("import-runner"),
                &repository(&conn_digest_mismatch, &root)
            )
            .is_err()
        );

        let conn_invalid_snapshot = conn();
        let manifest = manifest_value(&[PLANR_RUNNER_RESULT_FORMAT], &[RUNNER_OBSERVATION_TYPE]);
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        seed_verifier_with_instance(
            &conn_invalid_snapshot,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
            json!({"id": "instance-generic-adapter", "schema_version": "evidence.contract.v1", "manifest_id": "verifier-generic-adapter", "manifest_digest": manifest_digest, "host": "local", "surface": "local", "host_version": "test", "adapter_version": "1.0.0", "environment": {"kind": "local", "id": "env-local", "digest": "sha256:5555555555555555555555555555555555555555555555555555555555555555"}, "permissions": {"network": "none", "filesystem": "read"}, "availability": {"status": "available"}, "probe_result": {"probe_execution_id": "probe-1", "outcome": "passed", "observed_at": "2026-07-28T12:00:00Z", "checks": [{"name": "validator-available", "outcome": "passed"}]}, "observed_payload_contract": {"schema_ref": "schema://planr.runner.result", "observation_types": ["planr.runner.result"]}, "limitations": ["none"]}),
        );
        assert!(
            parse_validated_artifact_import(
                valid_runner_import("import-runner"),
                &repository(&conn_invalid_snapshot, &root)
            )
            .is_err()
        );

        let conn_generic = conn();
        seed_verifier(
            &conn_generic,
            &[GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT],
            &["planr.test.health"],
        );
        assert!(
            parse_validated_artifact_import(valid_import(), &repository(&conn_generic, &root))
                .is_err()
        );

        let conn_public_checksum = conn();
        seed_verifier(
            &conn_public_checksum,
            &[GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT],
            &["planr.test.health"],
        );
        let mut public_checksum = valid_import();
        let artifact_set_digest =
            public_checksum["adapter_predicate"]["attestation"]["artifact_set_digest"]
                .as_str()
                .unwrap();
        let predicate_digest =
            public_checksum["adapter_predicate"]["attestation"]["predicate_digest"]
                .as_str()
                .unwrap();
        public_checksum["adapter_predicate"]["attestation"] = json!({
            "kind": "planr_import_validator_attestation",
            "version": "1.0.0",
            "artifact_set_digest": artifact_set_digest,
            "predicate_digest": predicate_digest,
            "verifier_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "signature_digest": sha256_json_digest(&json!({
                "artifact_set_digest": artifact_set_digest,
                "predicate_digest": predicate_digest,
                "verifier_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            })).unwrap(),
        });
        assert!(
            parse_validated_artifact_import(
                public_checksum,
                &repository(&conn_public_checksum, &root)
            )
            .is_err()
        );

        let conn_bad_instance = conn();
        let manifest = manifest_value(&[PLANR_RUNNER_RESULT_FORMAT], &[RUNNER_OBSERVATION_TYPE]);
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        let mut bad_instance = instance_value(&manifest_digest, RUNNER_OBSERVATION_TYPE);
        bad_instance["adapter_version"] = json!("9.9.9");
        seed_verifier_with_instance(
            &conn_bad_instance,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
            bad_instance,
        );
        assert!(
            parse_validated_artifact_import(
                valid_runner_import("import-runner"),
                &repository(&conn_bad_instance, &root)
            )
            .is_err()
        );

        let conn_bad_payload = conn();
        let manifest = manifest_value(&[PLANR_RUNNER_RESULT_FORMAT], &[RUNNER_OBSERVATION_TYPE]);
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        let mut bad_payload_instance = instance_value(&manifest_digest, RUNNER_OBSERVATION_TYPE);
        bad_payload_instance["observed_payload_contract"]["observation_types"][0] =
            json!("planr.test.other");
        seed_verifier_with_instance(
            &conn_bad_payload,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
            bad_payload_instance,
        );
        assert!(
            parse_validated_artifact_import(
                valid_runner_import("import-runner"),
                &repository(&conn_bad_payload, &root)
            )
            .is_err()
        );
    }

    #[test]
    fn runner_validator_enforces_artifact_derived_status_invariants() {
        let root = fixture_root();
        let runner_conn = conn();
        seed_verifier(
            &runner_conn,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
        );
        let mut runner = valid_runner_import("import-runner");
        assert!(
            parse_validated_artifact_import(runner.clone(), &repository(&runner_conn, &root))
                .is_ok()
        );

        let ambiguous_conn = conn();
        seed_verifier(
            &ambiguous_conn,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
        );
        let (manifest, manifest_digest, execution_contract) = registered_manifest(&ambiguous_conn);
        insert_verifier_instance_row(
            &ambiguous_conn,
            &manifest,
            &manifest_digest,
            &unrelated_newer_instance_value(&manifest_digest, RUNNER_OBSERVATION_TYPE),
            &execution_contract,
        );
        assert!(
            parse_validated_artifact_import(
                valid_runner_import("import-runner-ambiguous-instance"),
                &repository(&ambiguous_conn, &root)
            )
            .is_err()
        );

        let mut relabelled_kind = valid_runner_import("import-runner-kind-forged");
        relabelled_kind["verifier_identity"]["kind"] = json!("host");
        assert!(
            parse_validated_artifact_import(relabelled_kind, &repository(&runner_conn, &root))
                .is_err()
        );

        let mut relabelled_name = valid_runner_import("import-runner-name-forged");
        relabelled_name["verifier_identity"]["name"] = json!("attacker-controlled-name");
        assert!(
            parse_validated_artifact_import(relabelled_name, &repository(&runner_conn, &root))
                .is_err()
        );

        runner["id"] = json!("import-runner-bad");
        runner["runner_result"]["status"] = json!("inconclusive");
        assert!(parse_validated_artifact_import(runner, &repository(&runner_conn, &root)).is_err());

        let mut relabelled_command = valid_runner_import("import-runner-command-forged");
        relabelled_command["runner_result"]["command"] = json!(["cargo", "test", "--release"]);
        assert!(
            parse_validated_artifact_import(relabelled_command, &repository(&runner_conn, &root))
                .is_err()
        );

        let mut relabelled_duration = valid_runner_import("import-runner-duration-forged");
        relabelled_duration["runner_result"]["duration_ms"] = json!(999);
        assert!(
            parse_validated_artifact_import(relabelled_duration, &repository(&runner_conn, &root))
                .is_err()
        );

        let failing_runner = r#"{"command":["cargo","test"],"exit_code":1,"status":"failed","stdout_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","stderr_digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","duration_ms":12}"#;
        let mut duplicate_runner = valid_runner_import("import-runner-pass-first-fail-second");
        duplicate_runner["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "artifact-runner-failing", "kind": "runner-json", "digest": digest(failing_runner), "uri": "file://runner-failing.json"}));
        assert!(
            parse_validated_artifact_import(duplicate_runner, &repository(&runner_conn, &root))
                .is_err()
        );

        let mut extra_runner = valid_runner_import("import-runner-extra-artifact");
        extra_runner["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "artifact-extra", "kind": "stdout-json", "digest": digest(r#"{"uninterpreted":true}"#), "uri": "file://extra-artifact.json"}));
        assert!(
            parse_validated_artifact_import(extra_runner, &repository(&runner_conn, &root))
                .is_err()
        );
    }

    #[test]
    fn junit_validator_derives_outcome_and_rejects_malformed_xml() {
        let root = fixture_root();
        let conn = conn();
        seed_verifier(&conn, &[JUNIT_XML_FORMAT], &[JUNIT_OBSERVATION_TYPE]);
        let mut junit = valid_import();
        junit["id"] = json!("import-junit");
        junit["format"] = json!(JUNIT_XML_FORMAT);
        junit.as_object_mut().unwrap().remove("adapter_predicate");
        junit["junit"] = json!({"kind": "junit_xml", "version": "1.0.0"});
        junit["artifact_refs"][0]["id"] = json!("artifact-junit");
        junit["artifact_refs"][0]["kind"] = json!("junit-xml");
        junit["artifact_refs"][0]["digest"] = json!(digest(
            r#"<testsuite name="unit" tests="1" failures="0" errors="0"><testcase name="passes" /></testsuite>"#
        ));
        junit["artifact_refs"][0]["uri"] = json!("file://test-results.xml");
        let passed =
            parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).unwrap();
        assert_eq!(passed.proposal.claims["junit"]["outcome"], "passed");
        assert_eq!(passed.proposal.claims["junit"]["skipped"], 0);

        let failing_junit_extra = r#"<testsuite name="unit" tests="1" failures="1" errors="0"><testcase name="fails"><failure /></testcase></testsuite>"#;
        let mut duplicate_junit = junit.clone();
        duplicate_junit["id"] = json!("import-junit-pass-first-fail-second");
        duplicate_junit["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "artifact-junit-failing", "kind": "junit-xml", "digest": digest(failing_junit_extra), "uri": "file://test-results-failing.xml"}));
        assert!(
            parse_validated_artifact_import(duplicate_junit, &repository(&conn, &root)).is_err()
        );

        let mut extra_junit = junit.clone();
        extra_junit["id"] = json!("import-junit-extra-artifact");
        extra_junit["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "artifact-extra", "kind": "stdout-json", "digest": digest(r#"{"uninterpreted":true}"#), "uri": "file://extra-artifact.json"}));
        assert!(parse_validated_artifact_import(extra_junit, &repository(&conn, &root)).is_err());

        let all_skipped = r#"<testsuite name="unit" tests="1" failures="0" errors="0" skipped="1"><testcase name="only"><skipped /></testcase></testsuite>"#;
        fs::write(root.path().join("test-results.xml"), all_skipped.as_bytes()).unwrap();
        junit["id"] = json!("import-junit-all-skipped");
        junit["artifact_refs"][0]["digest"] = json!(digest(all_skipped));
        let skipped =
            parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).unwrap();
        assert_eq!(skipped.proposal.claims["junit"]["outcome"], "skipped");
        assert_eq!(skipped.proposal.claims["junit"]["tests"], 1);
        assert_eq!(skipped.proposal.claims["junit"]["skipped"], 1);

        let mixed_skipped = r#"<testsuite name="unit" tests="2" failures="0" errors="0" skipped="1"><testcase name="passes" /><testcase name="skip"><skipped /></testcase></testsuite>"#;
        fs::write(
            root.path().join("test-results.xml"),
            mixed_skipped.as_bytes(),
        )
        .unwrap();
        junit["id"] = json!("import-junit-mixed-skipped");
        junit["artifact_refs"][0]["digest"] = json!(digest(mixed_skipped));
        let mixed =
            parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).unwrap();
        assert_eq!(mixed.proposal.claims["junit"]["outcome"], "passed");
        assert_eq!(mixed.proposal.claims["junit"]["tests"], 2);
        assert_eq!(mixed.proposal.claims["junit"]["skipped"], 1);

        let nested_skipped = r#"<testsuites tests="2" failures="0" errors="0" skipped="1"><testsuite name="unit" tests="1" failures="0" errors="0" skipped="0"><testcase name="passes" /></testsuite><testsuite name="integration" tests="1" failures="0" errors="0" skipped="1"><testcase name="skip"><skipped /></testcase></testsuite></testsuites>"#;
        fs::write(
            root.path().join("test-results.xml"),
            nested_skipped.as_bytes(),
        )
        .unwrap();
        junit["id"] = json!("import-junit-nested-skipped");
        junit["artifact_refs"][0]["digest"] = json!(digest(nested_skipped));
        let nested =
            parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).unwrap();
        assert_eq!(nested.proposal.claims["junit"]["outcome"], "passed");
        assert_eq!(nested.proposal.claims["junit"]["tests"], 2);
        assert_eq!(nested.proposal.claims["junit"]["skipped"], 1);

        fs::write(
            root.path().join("test-results.xml"),
            br#"<?xml version='1.0' encoding='UTF-8'?>
<!-- generated by a JUnit producer -->
<testsuites>
  <testsuite name='unit' tests='2' failures='1' errors='0'>
    <testcase classname='Example' name='fails'>
      <failure message='expected true'><![CDATA[stack trace <with> markup]]></failure>
      <system-out>ordinary stdout text</system-out>
      <system-err>ordinary stderr text</system-err>
    </testcase>
    <testcase classname='Example' name='passes' />
  </testsuite>
  <testsuite name='integration' tests='1' failures='0' errors='0'>
    <properties><property name='seed' value='42' /></properties>
    <testcase classname='Example' name='integrates' />
  </testsuite>
</testsuites>"#,
        )
        .unwrap();
        junit["id"] = json!("import-junit-failing");
        let failing_junit = fs::read_to_string(root.path().join("test-results.xml")).unwrap();
        junit["artifact_refs"][0]["digest"] = json!(digest(&failing_junit));
        let failed =
            parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).unwrap();
        assert_eq!(failed.proposal.claims["junit"]["outcome"], "failed");
        assert_eq!(failed.proposal.claims["junit"]["tests"], 3);

        let under_reported_failure = r#"<testsuite name="unit" tests="1" failures="0" errors="0"><testcase name="fails"><failure /></testcase></testsuite>"#;
        fs::write(
            root.path().join("test-results.xml"),
            under_reported_failure.as_bytes(),
        )
        .unwrap();
        junit["id"] = json!("import-junit-under-reported-failure");
        junit["artifact_refs"][0]["digest"] = json!(digest(under_reported_failure));
        assert!(parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).is_err());

        let under_reported_error = r#"<testsuite name="unit" tests="1" failures="0" errors="0"><testcase name="errors"><error /></testcase></testsuite>"#;
        fs::write(
            root.path().join("test-results.xml"),
            under_reported_error.as_bytes(),
        )
        .unwrap();
        junit["id"] = json!("import-junit-under-reported-error");
        junit["artifact_refs"][0]["digest"] = json!(digest(under_reported_error));
        assert!(parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).is_err());

        for forged_counts in [
            r#"<testsuite name="unit" tests="999" failures="0" errors="0" />"#,
            r#"<testsuite name="unit" tests="1" failures="999" errors="0"><testcase name="passes" /></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="0" errors="999"><testcase name="passes" /></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="0" errors="0" skipped="0"><testcase name="skip"><skipped /></testcase></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="0" errors="0" skipped="1"><testcase name="passes" /></testsuite>"#,
            r#"<testsuite tests="1" failures="0" errors="0" skipped="1" skipped="0"><testcase name="only"><skipped /></testcase></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="0" errors="0" skipped="2"><testcase name="only"><skipped /><skipped /></testcase></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="1" errors="0" skipped="1"><testcase name="conflict"><failure /><skipped /></testcase></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="0" errors="1" skipped="1"><testcase name="conflict"><error /><skipped /></testcase></testsuite>"#,
            r#"<testsuite name="unit" tests="1" failures="1" errors="1" skipped="0"><testcase name="conflict"><failure /><error /></testcase></testsuite>"#,
            r#"<testsuites tests="4" failures="0" errors="0"><testsuite name="aggregate" tests="2" failures="0" errors="0"><testsuite name="child" tests="2" failures="0" errors="0"><testcase name="one" /><testcase name="two" /></testsuite></testsuite></testsuites>"#,
            r#"<testsuite name="unit" tests="18446744073709551616" failures="0" errors="0"><testcase name="passes" /></testsuite>"#,
        ] {
            fs::write(
                root.path().join("test-results.xml"),
                forged_counts.as_bytes(),
            )
            .unwrap();
            junit["id"] = json!(format!(
                "import-junit-forged-counts-{}",
                forged_counts.len()
            ));
            junit["artifact_refs"][0]["digest"] = json!(digest(forged_counts));
            assert!(
                parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).is_err()
            );
        }

        for malformed in [
            r#"noise <testsuite tests="1" failures="0" errors="0"></testsuite>"#,
            r#"<testsuite tests="1" failures="0" errors="0"></testsuites>"#,
            r#"<testsuite tests="1" failures="0" errors="0"></testsuite><testsuite tests="1" failures="0" errors="0"></testsuite>"#,
            r#"<testsuites></testsuites>"#,
            r#"<testsuites><testcase name="direct-root" /></testsuites>"#,
            r#"<testsuite tests="1"><properties><testcase name="misnested" /></properties></testsuite>"#,
            r#"<testsuite tests="0" failures="1"><failure /></testsuite>"#,
            r#"<testsuite tests="0" errors="1"><error /></testsuite>"#,
            r#"<testsuite tests="1" failures="1"><testcase name="fails"><properties><failure /></properties></testcase></testsuite>"#,
            r#"<testsuite tests="1" errors="1"><testcase name="errors"><properties><error /></properties></testcase></testsuite>"#,
        ] {
            fs::write(root.path().join("test-results.xml"), malformed.as_bytes()).unwrap();
            junit["id"] = json!(format!("import-junit-malformed-{}", malformed.len()));
            junit["artifact_refs"][0]["digest"] = json!(digest(malformed));
            assert!(
                parse_validated_artifact_import(junit.clone(), &repository(&conn, &root)).is_err()
            );
        }
    }

    #[test]
    fn validated_import_persists_restart_idempotence_and_rejects_conflict() {
        let root = fixture_root();
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("imports.sqlite");
        {
            let conn = crate::storage::open_db(&db_path).unwrap();
            crate::storage::ensure_schema(&conn).unwrap();
            conn.execute_batch(
                "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
                 VALUES ('p-evidence', 'Evidence', '.', 'active', datetime('now'), datetime('now'))",
            )
            .unwrap();
            seed_verifier(
                &conn,
                &[PLANR_RUNNER_RESULT_FORMAT],
                &[RUNNER_OBSERVATION_TYPE],
            );
            let first = parse_validated_artifact_import(
                valid_runner_import("import-runner"),
                &repository(&conn, &root),
            )
            .unwrap();
            assert!(!first.idempotent);
        }
        {
            let conn = crate::storage::open_db(&db_path).unwrap();
            let second = parse_validated_artifact_import(
                valid_runner_import("import-runner"),
                &repository(&conn, &root),
            )
            .unwrap();
            assert!(second.idempotent);
            let mut conflicting = valid_runner_import("import-runner");
            conflicting["submitted_at"] = json!("2026-07-28T12:00:01Z");
            assert!(
                parse_validated_artifact_import(conflicting, &repository(&conn, &root)).is_err()
            );
        }

        let same_digest_db_path = temp.path().join("imports-same-digest-race.sqlite");
        seed_runner_import_database(&same_digest_db_path);
        let same_digest_results = race_runner_imports(
            same_digest_db_path,
            root.path().to_path_buf(),
            valid_runner_import("import-runner"),
            valid_runner_import("import-runner"),
        );
        let mut same_digest_idempotent = same_digest_results
            .into_iter()
            .map(|result| result.unwrap())
            .collect::<Vec<_>>();
        same_digest_idempotent.sort();
        assert_eq!(same_digest_idempotent, vec![false, true]);

        let conflicting_db_path = temp.path().join("imports-conflicting-digest-race.sqlite");
        seed_runner_import_database(&conflicting_db_path);
        let mut conflicting = valid_runner_import("import-runner");
        conflicting["submitted_at"] = json!("2026-07-28T12:00:01Z");
        let conflicting_results = race_runner_imports(
            conflicting_db_path,
            root.path().to_path_buf(),
            valid_runner_import("import-runner"),
            conflicting,
        );
        let successes = conflicting_results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect::<Vec<_>>();
        let failures = conflicting_results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .collect::<Vec<_>>();
        assert_eq!(successes, vec![&false]);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("invalid Planr-assigned binding import.id"));
    }

    fn seed_runner_import_database(db_path: &Path) {
        let conn = crate::storage::open_db(db_path).unwrap();
        crate::storage::ensure_schema(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-evidence', 'Evidence', '.', 'active', datetime('now'), datetime('now'))",
        )
        .unwrap();
        seed_verifier(
            &conn,
            &[PLANR_RUNNER_RESULT_FORMAT],
            &[RUNNER_OBSERVATION_TYPE],
        );
    }

    fn race_runner_imports(
        db_path: PathBuf,
        artifact_root: PathBuf,
        left: Value,
        right: Value,
    ) -> Vec<Result<bool, String>> {
        let barrier = Arc::new(Barrier::new(2));
        [left, right]
            .into_iter()
            .map(|import| {
                let db_path = db_path.clone();
                let artifact_root = artifact_root.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let conn = crate::storage::open_db(&db_path).map_err(|err| err.to_string())?;
                    barrier.wait();
                    parse_validated_artifact_import(
                        import,
                        &repository_for_path(&conn, &artifact_root),
                    )
                    .map(|record| record.idempotent)
                    .map_err(|err| err.to_string())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    }
}
