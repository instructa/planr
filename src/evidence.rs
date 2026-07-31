pub(crate) mod adapter_signal;
pub(crate) mod adapters;
pub mod coverage;
pub(crate) mod execution;
mod import;
pub(crate) mod model;
pub(crate) mod policy;
pub(crate) mod registry;

use serde::{Deserialize, Deserializer, de};
use serde_json::{Map, Value};
use std::str::FromStr;

#[allow(unused_imports)]
pub use import::{
    GENERIC_VERSIONED_ADAPTER_PREDICATE_FORMAT, JUNIT_XML_FORMAT, PLANR_RUNNER_RESULT_FORMAT,
    VALIDATED_ARTIFACT_IMPORT_SCHEMA_VERSION, ValidatedArtifactImportRepository,
    ValidatedImportRecord, parse_validated_artifact_import,
};
#[allow(unused_imports)]
pub use model::{
    AttemptStatus, CapabilityAvailabilityStatus, CoverageObservationStatus, CoverageStatus,
    EvidenceDomainError, EvidenceId, GapReason, NamespacedIdentifier, ProvenanceSourceKind,
    ReceiptStatus, SchemaVersion, SourceKind,
};
#[allow(unused_imports)]
pub(crate) use model::{
    EnvironmentBinding, FixtureDisclosure, ProcessExecutionContract, ProofObligation,
    TargetBinding, VerificationCapabilityInstance, VerificationCapabilityManifest,
};
#[allow(unused_imports)]
pub(crate) use registry::{CapabilityRegistry, CapabilityRuntimeContext};

#[derive(Debug, Clone)]
pub struct UntrustedArtifactRef {
    pub id: String,
    pub kind: String,
    pub digest: String,
    pub uri: Option<String>,
    pub extra: Map<String, Value>,
}

impl UntrustedArtifactRef {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        EvidenceId::parse(self.id.clone())?;
        if self.kind.is_empty() {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "artifact_refs[].kind",
            ));
        }
        model::Sha256Digest::parse(self.digest.clone())?;
        if self.uri.as_deref() == Some("") {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "artifact_refs[].uri",
            ));
        }
        reject_forbidden_authority_fields(&self.extra)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for UntrustedArtifactRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            kind: String,
            digest: String,
            #[serde(default, deserialize_with = "deserialize_optional_string_without_null")]
            uri: Option<String>,
            #[serde(flatten)]
            extra: Map<String, Value>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let artifact_ref = Self {
            id: raw.id,
            kind: raw.kind,
            digest: raw.digest,
            uri: raw.uri,
            extra: raw.extra,
        };
        artifact_ref
            .validate()
            .map_err(|err| de::Error::custom(err.to_string()))?;
        Ok(artifact_ref)
    }
}

#[derive(Debug, Clone)]
pub struct UntrustedEvidenceProposal {
    pub id: String,
    pub schema_version: String,
    pub source_kind: String,
    pub submitted_at: String,
    pub claims: Map<String, Value>,
    pub artifact_refs: Vec<UntrustedArtifactRef>,
    pub producer_metadata: Map<String, Value>,
}

impl UntrustedEvidenceProposal {
    pub fn validate(&self) -> Result<(), EvidenceDomainError> {
        SchemaVersion::deserialize(Value::String(self.schema_version.clone()))
            .map_err(|err| EvidenceDomainError::InvalidSchemaVersion(err.to_string()))?;
        EvidenceId::parse(self.id.clone())?;
        SourceKind::from_str(&self.source_kind)?;
        model::validate_timestamp(&self.submitted_at)?;
        for artifact_ref in &self.artifact_refs {
            artifact_ref.validate()?;
        }
        reject_forbidden_authority_fields(&self.claims)?;
        reject_forbidden_authority_fields(&self.producer_metadata)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for UntrustedEvidenceProposal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: String,
            schema_version: String,
            source_kind: String,
            submitted_at: String,
            claims: Map<String, Value>,
            artifact_refs: Vec<UntrustedArtifactRef>,
            producer_metadata: Map<String, Value>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let proposal = Self {
            id: raw.id,
            schema_version: raw.schema_version,
            source_kind: raw.source_kind,
            submitted_at: raw.submitted_at,
            claims: raw.claims,
            artifact_refs: raw.artifact_refs,
            producer_metadata: raw.producer_metadata,
        };
        proposal
            .validate()
            .map_err(|err| de::Error::custom(err.to_string()))?;
        Ok(proposal)
    }
}

#[allow(dead_code)]
pub fn parse_untrusted_evidence_proposal(
    value: Value,
) -> Result<UntrustedEvidenceProposal, serde_json::Error> {
    serde_json::from_value(value)
}

#[allow(dead_code)]
pub fn parse_verification_capability_instance(value: Value) -> Result<(), EvidenceDomainError> {
    serde_json::from_value::<model::VerificationCapabilityInstance>(value)
        .map(|_| ())
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("capability_instance_json"))
}

#[allow(dead_code)]
pub fn trusted_receipt_binding_matches_receipt(
    trusted_binding: Value,
    receipt: Value,
) -> Result<(), EvidenceDomainError> {
    let receipt = model::EvidenceReceipt::from_trusted_value(receipt)?;
    let binding: model::TrustedReceiptBinding = serde_json::from_value(trusted_binding)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("trusted_binding_json"))?;
    binding.validate_receipt_exact(&receipt)
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

pub(crate) fn reject_forbidden_authority_fields(
    fields: &Map<String, Value>,
) -> Result<(), EvidenceDomainError> {
    for field in [
        "provenance",
        "execution_id",
        "receipt_digest",
        "closure_authority",
        "receipt_status",
        "attempt_ids",
        "retry_history",
        "raw_result",
        "source",
        "source_revision",
        "tree_digest",
        "target",
        "target_digest",
        "environment",
        "environment_identity",
        "environment_digest",
        "vantage_point",
        "capability",
        "manifest_id",
        "manifest_digest",
        "instance_id",
        "instance_digest",
        "config_digest",
        "permissions",
        "sandbox",
        "coverage_result",
        "freshness_result",
        "policy_result",
    ] {
        if fields.contains_key(field) {
            return Err(EvidenceDomainError::ForbiddenTrustedField(field));
        }
    }
    for value in fields.values() {
        reject_forbidden_authority_value(value)?;
    }
    Ok(())
}

pub(crate) fn reject_forbidden_authority_value(value: &Value) -> Result<(), EvidenceDomainError> {
    match value {
        Value::Object(fields) => reject_forbidden_authority_fields(fields),
        Value::Array(values) => {
            for value in values {
                reject_forbidden_authority_value(value)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_untrusted_boundary_rejects_nested_authority_fields_and_artifact_extras() {
        let valid = json!({
            "id": "proposal-public",
            "schema_version": "evidence.contract.v1",
            "source_kind": "agent",
            "submitted_at": "2026-07-28T12:00:00Z",
            "claims": {"summary": "public input"},
            "artifact_refs": [{
                "id": "artifact-public",
                "kind": "log",
                "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }],
            "producer_metadata": {"client": "compile-probe"}
        });

        let mut nested = valid.clone();
        nested["claims"]["nested"] = json!({"target_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"});
        assert!(parse_untrusted_evidence_proposal(nested).is_err());

        let mut artifact_extra = valid;
        artifact_extra["artifact_refs"][0]["manifest_digest"] =
            json!("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
        assert!(parse_untrusted_evidence_proposal(artifact_extra).is_err());
    }
}
