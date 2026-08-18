#![allow(dead_code)]

use crate::canonical_json::sha256_json_digest_without_top_level_field;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value};
use std::fmt;
use std::str::FromStr;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const EVIDENCE_CONTRACT_V1: &str = "evidence.contract.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceDomainError {
    InvalidSchemaVersion(String),
    InvalidIdentifier(String),
    InvalidNamespacedIdentifier(String),
    InvalidDigest(String),
    InvalidTimestamp(String),
    InvalidTrustedBinding(&'static str),
    InvalidStatus { kind: &'static str, value: String },
    ForbiddenTrustedField(&'static str),
    MissingTrustedBinding(&'static str),
    Digest(String),
}

impl fmt::Display for EvidenceDomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchemaVersion(value) => {
                write!(
                    f,
                    "schema_version must be {EVIDENCE_CONTRACT_V1}, got {value}"
                )
            }
            Self::InvalidIdentifier(value) => write!(f, "invalid Evidence identifier: {value}"),
            Self::InvalidNamespacedIdentifier(value) => {
                write!(f, "invalid Evidence namespaced identifier: {value}")
            }
            Self::InvalidDigest(value) => {
                write!(f, "digest must be sha256:<64 lowercase hex>, got {value}")
            }
            Self::InvalidTimestamp(value) => {
                write!(
                    f,
                    "timestamp must be non-empty RFC 3339 date-time, got {value}"
                )
            }
            Self::InvalidTrustedBinding(field) => {
                write!(
                    f,
                    "trusted receipt has invalid Planr-assigned binding {field}"
                )
            }
            Self::InvalidStatus { kind, value } => {
                write!(f, "invalid {kind} status: {value}")
            }
            Self::ForbiddenTrustedField(field) => {
                write!(f, "untrusted evidence cannot include trusted field {field}")
            }
            Self::MissingTrustedBinding(field) => {
                write!(
                    f,
                    "trusted receipt is missing Planr-assigned binding {field}"
                )
            }
            Self::Digest(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for EvidenceDomainError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SchemaVersion(String);

impl SchemaVersion {
    pub fn v1() -> Self {
        Self(EVIDENCE_CONTRACT_V1.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == EVIDENCE_CONTRACT_V1 {
            Ok(Self(value))
        } else {
            Err(serde::de::Error::custom(
                EvidenceDomainError::InvalidSchemaVersion(value),
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct EvidenceId(String);

impl EvidenceId {
    pub fn parse(value: impl Into<String>) -> Result<Self, EvidenceDomainError> {
        let value = value.into();
        if is_valid_identifier(&value) {
            Ok(Self(value))
        } else {
            Err(EvidenceDomainError::InvalidIdentifier(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for EvidenceId {
    type Err = EvidenceDomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for EvidenceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct NamespacedIdentifier(String);

impl NamespacedIdentifier {
    pub fn parse(value: impl Into<String>) -> Result<Self, EvidenceDomainError> {
        let value = value.into();
        if is_valid_namespaced_identifier(&value) {
            Ok(Self(value))
        } else {
            Err(EvidenceDomainError::InvalidNamespacedIdentifier(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for NamespacedIdentifier {
    type Err = EvidenceDomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for NamespacedIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct Sha256Digest(String);

impl Sha256Digest {
    pub(crate) fn parse(value: impl Into<String>) -> Result<Self, EvidenceDomainError> {
        let value = value.into();
        if is_sha256_digest(&value) {
            Ok(Self(value))
        } else {
            Err(EvidenceDomainError::InvalidDigest(value))
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

macro_rules! exact_status_enum {
    ($name:ident, $kind:literal, { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl FromStr for $name {
            type Err = EvidenceDomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(EvidenceDomainError::InvalidStatus {
                        kind: $kind,
                        value: other.to_string(),
                    }),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::from_str(&String::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

exact_status_enum!(AttemptStatus, "EvidenceAttempt", {
    Passed => "passed",
    Failed => "failed",
    Skipped => "skipped",
    TimedOut => "timed_out",
    Aborted => "aborted",
    Unavailable => "unavailable",
    Inconclusive => "inconclusive",
});

exact_status_enum!(ReceiptStatus, "EvidenceReceipt", {
    Trusted => "trusted",
    Rejected => "rejected",
    Untrusted => "untrusted",
    Stale => "stale",
    Superseded => "superseded",
});

exact_status_enum!(CapabilityAvailabilityStatus, "VerificationCapabilityInstance", {
    Available => "available",
    Unavailable => "unavailable",
    Degraded => "degraded",
    PermissionDenied => "permission_denied",
    SandboxBlocked => "sandbox_blocked",
    Unsupported => "unsupported",
    ProbeFailed => "probe_failed",
});

exact_status_enum!(CoverageStatus, "CoverageVerdict", {
    Satisfied => "satisfied",
    Unsatisfied => "unsatisfied",
    Blocked => "blocked",
    Inconclusive => "inconclusive",
    Waived => "waived",
    Stale => "stale",
});

exact_status_enum!(CoverageObservationStatus, "CoverageObservation", {
    Covered => "covered",
    Missing => "missing",
    Unsatisfied => "unsatisfied",
    Blocked => "blocked",
    Inconclusive => "inconclusive",
    Waived => "waived",
    Stale => "stale",
});

exact_status_enum!(SourceKind, "UntrustedEvidenceProposal.source_kind", {
    Agent => "agent",
    Adapter => "adapter",
    Host => "host",
    Mcp => "mcp",
    ArtifactImport => "artifact_import",
    User => "user",
});

exact_status_enum!(ProvenanceSourceKind, "TrustedProvenance.source", {
    PlanrObservedExecution => "planr_observed_execution",
    VerifiedHostEvent => "verified_host_event",
    McpAttestation => "mcp_attestation",
    ValidatedArtifactImport => "validated_artifact_import",
    UserAttestation => "user_attestation",
});

exact_status_enum!(AdapterKind, "VerificationCapabilityManifest.adapter_kind", {
    Process => "process",
    Host => "host",
    Mcp => "mcp",
    ArtifactImport => "artifact_import",
    UserAttestation => "user_attestation",
});

exact_status_enum!(GapReason, "CoverageGapReason", {
    MissingObservation => "missing_observation",
    MissingCapability => "missing_capability",
    PermissionDenied => "permission_denied",
    SandboxBlocked => "sandbox_blocked",
    EnvironmentUnavailable => "environment_unavailable",
    ExternalDependencyUnavailable => "external_dependency_unavailable",
    ProductFailed => "product_failed",
    VerifierFailed => "verifier_failed",
    TimedOut => "timed_out",
    Aborted => "aborted",
    InconclusiveResult => "inconclusive_result",
    StaleSource => "stale_source",
    StaleTarget => "stale_target",
    StaleEnvironment => "stale_environment",
    StalePolicy => "stale_policy",
    StaleAdapterSchema => "stale_adapter_schema",
    StaleConfiguration => "stale_configuration",
    TargetMismatch => "target_mismatch",
    SchemaMismatch => "schema_mismatch",
    ManifestMismatch => "manifest_mismatch",
    UntrustedProvenance => "untrusted_provenance",
    FixtureDisallowed => "fixture_disallowed",
    MockDisallowed => "mock_disallowed",
    InsufficientAssurance => "insufficient_assurance",
    WaiverMissing => "waiver_missing",
    WaiverExpired => "waiver_expired",
    UnknownObservationType => "unknown_observation_type",
    UnsupportedRuntimeTarget => "unsupported_runtime_target",
});

impl GapReason {
    pub const LEGACY_ALIASES: &'static [(&'static str, Self)] = &[
        ("capability_unavailable", Self::MissingCapability),
        (
            "dependency_unavailable",
            Self::ExternalDependencyUnavailable,
        ),
        ("policy_failed", Self::StalePolicy),
        ("trust_failed", Self::UntrustedProvenance),
        ("stale_evidence", Self::StaleSource),
    ];

    pub fn canonicalize(value: &str) -> Self {
        if let Ok(reason) = Self::from_str(value) {
            return reason;
        }
        Self::LEGACY_ALIASES
            .iter()
            .find_map(|(alias, reason)| (*alias == value).then_some(*reason))
            .unwrap_or(Self::VerifierFailed)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationRequirement {
    pub id: EvidenceId,
    #[serde(rename = "type")]
    pub observation_type: NamespacedIdentifier,
    pub subject: String,
    pub expected: Value,
    pub target: Value,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub payload_schema: Option<SchemaReferenceBinding>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub state_transitions: Option<Value>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub persistence: Option<Value>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub negative_assertions: Option<Value>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub freshness_policy: Option<Value>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub assurance_policy: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SchemaReferenceBinding {
    pub schema_ref: String,
}

impl SchemaReferenceBinding {
    fn validate(&self, field: &'static str) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.schema_ref, field)
    }
}

impl<'de> Deserialize<'de> for SchemaReferenceBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_ref: String,
        }

        let binding = Self {
            schema_ref: Raw::deserialize(deserializer)?.schema_ref,
        };
        binding
            .validate("payload_schema.schema_ref")
            .map_err(serde::de::Error::custom)?;
        Ok(binding)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadSchemaBinding {
    #[serde(rename = "type")]
    pub observation_type: NamespacedIdentifier,
    pub schema_ref: String,
    pub schema_digest: Sha256Digest,
}

impl PayloadSchemaBinding {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.schema_ref, "payload_schema.schema_ref")
    }
}

impl<'de> Deserialize<'de> for PayloadSchemaBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(rename = "type")]
            observation_type: NamespacedIdentifier,
            schema_ref: String,
            schema_digest: Sha256Digest,
        }

        let raw = Raw::deserialize(deserializer)?;
        let binding = Self {
            observation_type: raw.observation_type,
            schema_ref: raw.schema_ref,
            schema_digest: raw.schema_digest,
        };
        binding.validate().map_err(serde::de::Error::custom)?;
        Ok(binding)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofObligation {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub criterion_id: EvidenceId,
    pub plan_id: EvidenceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<EvidenceId>,
    pub title: String,
    pub binding: bool,
    pub observations: Vec<ObservationRequirement>,
    pub fixture_policy: Value,
    pub freshness_policy: Value,
    pub assurance_policy: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<EvidenceId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerificationCapabilityManifest {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub version: String,
    pub adapter_kind: AdapterKind,
    pub adapter_digest: Sha256Digest,
    pub supported_surfaces: Vec<String>,
    pub supported_observations: Vec<PayloadSchemaBinding>,
    pub supported_interactions: Vec<String>,
    pub supported_artifacts: Vec<String>,
    pub runtime_targets: Vec<RuntimeTarget>,
    pub provenance_path: ProvenanceSourceKind,
    pub permissions: Map<String, Value>,
    pub costs: Map<String, Value>,
    pub determinism: String,
    pub repeatability: String,
    pub independence: String,
    pub blind_spots: Vec<String>,
    pub availability_probe: AvailabilityProbeContract,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationCapabilityManifestRaw {
    id: EvidenceId,
    schema_version: SchemaVersion,
    version: String,
    adapter_kind: AdapterKind,
    adapter_digest: Sha256Digest,
    supported_surfaces: Vec<String>,
    supported_observations: Vec<PayloadSchemaBinding>,
    supported_interactions: Vec<String>,
    supported_artifacts: Vec<String>,
    runtime_targets: Vec<RuntimeTarget>,
    provenance_path: ProvenanceSourceKind,
    permissions: Map<String, Value>,
    costs: Map<String, Value>,
    determinism: String,
    repeatability: String,
    independence: String,
    blind_spots: Vec<String>,
    availability_probe: AvailabilityProbeContract,
}

impl TryFrom<VerificationCapabilityManifestRaw> for VerificationCapabilityManifest {
    type Error = EvidenceDomainError;

    fn try_from(raw: VerificationCapabilityManifestRaw) -> Result<Self, Self::Error> {
        require_non_empty(&raw.version, "version")?;
        validate_string_list(&raw.supported_surfaces, "supported_surfaces")?;
        if raw.supported_observations.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding(
                "supported_observations",
            ));
        }
        validate_string_list(&raw.supported_interactions, "supported_interactions")?;
        validate_string_list(&raw.supported_artifacts, "supported_artifacts")?;
        if raw.runtime_targets.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding(
                "runtime_targets",
            ));
        }
        for runtime_target in &raw.runtime_targets {
            runtime_target.validate()?;
        }
        require_non_empty(&raw.determinism, "determinism")?;
        require_non_empty(&raw.repeatability, "repeatability")?;
        require_non_empty(&raw.independence, "independence")?;
        validate_string_list(&raw.blind_spots, "blind_spots")?;
        raw.availability_probe.validate()?;
        Ok(Self {
            id: raw.id,
            schema_version: raw.schema_version,
            version: raw.version,
            adapter_kind: raw.adapter_kind,
            adapter_digest: raw.adapter_digest,
            supported_surfaces: raw.supported_surfaces,
            supported_observations: raw.supported_observations,
            supported_interactions: raw.supported_interactions,
            supported_artifacts: raw.supported_artifacts,
            runtime_targets: raw.runtime_targets,
            provenance_path: raw.provenance_path,
            permissions: raw.permissions,
            costs: raw.costs,
            determinism: raw.determinism,
            repeatability: raw.repeatability,
            independence: raw.independence,
            blind_spots: raw.blind_spots,
            availability_probe: raw.availability_probe,
        })
    }
}

impl<'de> Deserialize<'de> for VerificationCapabilityManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        VerificationCapabilityManifestRaw::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct RuntimeTarget {
    pub kind: String,
    pub id: EvidenceId,
    #[serde(default, flatten, skip_serializing_if = "Map::is_empty")]
    pub extensions: Map<String, Value>,
}

impl RuntimeTarget {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.kind, "runtime_targets[].kind")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AvailabilityProbeContract {
    pub kind: String,
    pub execution: ProcessExecutionContract,
}

impl AvailabilityProbeContract {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        if self.kind != "process" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "availability_probe.kind",
            ));
        }
        self.execution.validate()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessExecutionContract {
    pub kind: String,
    pub executable: String,
    pub args: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub working_directory: Option<String>,
    pub timeout_ms: u64,
    pub stdout_limit_bytes: u64,
    pub stderr_limit_bytes: u64,
    pub payload_schema: PayloadSchemaBinding,
}

impl ProcessExecutionContract {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        if self.kind != "process" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "availability_probe.execution.kind",
            ));
        }
        require_non_empty(&self.executable, "availability_probe.execution.executable")?;
        if let Some(working_directory) = &self.working_directory {
            require_non_empty(
                working_directory,
                "availability_probe.execution.working_directory",
            )?;
        }
        if self.timeout_ms < 1 {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "availability_probe.execution.timeout_ms",
            ));
        }
        if self.stdout_limit_bytes < 1 {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "availability_probe.execution.stdout_limit_bytes",
            ));
        }
        if self.stderr_limit_bytes < 1 {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "availability_probe.execution.stderr_limit_bytes",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerificationCapabilityInstance {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub manifest_id: EvidenceId,
    pub manifest_digest: Sha256Digest,
    pub host: String,
    pub surface: String,
    pub host_version: String,
    pub adapter_version: String,
    pub environment: EnvironmentBinding,
    pub permissions: PermissionState,
    pub availability: CapabilityAvailability,
    pub probe_result: ProbeResult,
    pub observed_payload_contract: ObservedPayloadContract,
    pub limitations: Vec<String>,
    pub captured_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityAvailability {
    pub status: CapabilityAvailabilityStatus,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub reason: Option<String>,
}

impl CapabilityAvailability {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        if let Some(reason) = &self.reason {
            require_non_empty(reason, "availability.reason")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeResult {
    pub probe_execution_id: EvidenceId,
    pub outcome: AttemptStatus,
    pub observed_at: String,
    pub checks: Vec<ProbeCheck>,
}

impl ProbeResult {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        validate_timestamp(&self.observed_at)?;
        if self.checks.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding(
                "probe_result.checks",
            ));
        }
        for check in &self.checks {
            check.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProbeCheck {
    pub name: String,
    pub outcome: AttemptStatus,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub detail: Option<String>,
}

impl ProbeCheck {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.name, "probe_result.checks[].name")?;
        if let Some(detail) = &self.detail {
            require_non_empty(detail, "probe_result.checks[].detail")?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationCapabilityInstanceRaw {
    id: EvidenceId,
    schema_version: SchemaVersion,
    manifest_id: EvidenceId,
    manifest_digest: Sha256Digest,
    host: String,
    surface: String,
    host_version: String,
    adapter_version: String,
    environment: EnvironmentBinding,
    permissions: PermissionState,
    availability: CapabilityAvailability,
    probe_result: ProbeResult,
    observed_payload_contract: ObservedPayloadContract,
    limitations: Vec<String>,
    captured_at: String,
}

impl TryFrom<VerificationCapabilityInstanceRaw> for VerificationCapabilityInstance {
    type Error = EvidenceDomainError;

    fn try_from(raw: VerificationCapabilityInstanceRaw) -> Result<Self, Self::Error> {
        require_non_empty(&raw.host, "host")?;
        require_non_empty(&raw.surface, "surface")?;
        require_non_empty(&raw.host_version, "host_version")?;
        require_non_empty(&raw.adapter_version, "adapter_version")?;
        raw.environment.validate()?;
        raw.permissions.validate()?;
        raw.availability.validate()?;
        raw.probe_result.validate()?;
        if raw.availability.status == CapabilityAvailabilityStatus::Available
            && raw.probe_result.outcome != AttemptStatus::Passed
        {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "probe_result.outcome",
            ));
        }
        raw.observed_payload_contract.validate()?;
        validate_optional_string_list(Some(&raw.limitations), "limitations")?;
        validate_timestamp(&raw.captured_at)?;
        Ok(Self {
            id: raw.id,
            schema_version: raw.schema_version,
            manifest_id: raw.manifest_id,
            manifest_digest: raw.manifest_digest,
            host: raw.host,
            surface: raw.surface,
            host_version: raw.host_version,
            adapter_version: raw.adapter_version,
            environment: raw.environment,
            permissions: raw.permissions,
            availability: raw.availability,
            probe_result: raw.probe_result,
            observed_payload_contract: raw.observed_payload_contract,
            limitations: raw.limitations,
            captured_at: raw.captured_at,
        })
    }
}

impl<'de> Deserialize<'de> for VerificationCapabilityInstance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        VerificationCapabilityInstanceRaw::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservedPayloadContract {
    pub schema_ref: String,
    pub observation_types: Vec<NamespacedIdentifier>,
}

impl ObservedPayloadContract {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.schema_ref, "observed_payload_contract.schema_ref")?;
        if self.observation_types.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding(
                "observed_payload_contract.observation_types",
            ));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ObservedPayloadContract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_ref: String,
            observation_types: Vec<NamespacedIdentifier>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let contract = Self {
            schema_ref: raw.schema_ref,
            observation_types: raw.observation_types,
        };
        contract.validate().map_err(serde::de::Error::custom)?;
        Ok(contract)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceAttempt {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub criterion_id: EvidenceId,
    pub obligation_id: EvidenceId,
    pub capability_instance_id: EvidenceId,
    pub started_at: String,
    pub ended_at: String,
    pub status: AttemptStatus,
    pub resolved_command: Value,
    pub exit: Value,
    pub retry_lineage: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_binding: Option<Value>,
    pub stdout_digest: Sha256Digest,
    pub stderr_digest: Sha256Digest,
    pub raw_result: Value,
    pub artifacts: Vec<Value>,
    pub output_bounds: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoverageVerdict {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub scope: Value,
    pub evaluated_at: String,
    pub status: CoverageStatus,
    pub observation_coverage: Vec<Value>,
    pub validation_details: Value,
    pub suggested_next_action: String,
    pub actionable_now: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidencePolicy {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub policy_digest: Sha256Digest,
    pub defaults: Value,
    pub named_presets: Vec<ProofPreset>,
    pub observation_schema_registrations: Vec<Value>,
    pub adapter_registrations: Vec<Value>,
    pub extension_namespaces: Vec<NamespacedIdentifier>,
    pub trust_policy: Value,
    pub freshness_policy: Value,
    pub fixture_policy: Value,
    pub completion_policy: Value,
    pub layering_policy: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofPreset {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub namespace: NamespacedIdentifier,
    pub observations: Vec<ObservationRequirement>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceWaiver {
    pub id: EvidenceId,
    pub schema_version: SchemaVersion,
    pub scope: EvidenceScope,
    pub observation_ids: Vec<EvidenceId>,
    pub source: SourceBinding,
    pub target: TargetBinding,
    pub reason: String,
    pub created_by: String,
    pub created_at: String,
    pub expires_at: String,
    pub approval_ref: EvidenceId,
    pub audit_trail: Vec<Map<String, Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceScope {
    pub kind: String,
    pub id: EvidenceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<EvidenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<EvidenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion_id: Option<EvidenceId>,
}

impl EvidenceScope {
    pub(crate) fn validate(&self) -> Result<(), EvidenceDomainError> {
        if matches!(self.kind.as_str(), "criterion" | "item" | "plan" | "goal") {
            Ok(())
        } else {
            Err(EvidenceDomainError::InvalidTrustedBinding("scope.kind"))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceWaiverRaw {
    id: EvidenceId,
    schema_version: SchemaVersion,
    scope: EvidenceScope,
    observation_ids: Vec<EvidenceId>,
    source: SourceBinding,
    target: TargetBinding,
    reason: String,
    created_by: String,
    created_at: String,
    expires_at: String,
    approval_ref: EvidenceId,
    audit_trail: Vec<Map<String, Value>>,
}

impl TryFrom<EvidenceWaiverRaw> for EvidenceWaiver {
    type Error = EvidenceDomainError;

    fn try_from(raw: EvidenceWaiverRaw) -> Result<Self, Self::Error> {
        raw.scope.validate()?;
        if raw.observation_ids.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding(
                "observation_ids",
            ));
        }
        raw.source.validate()?;
        raw.target.validate()?;
        require_non_empty(&raw.reason, "reason")?;
        require_non_empty(&raw.created_by, "created_by")?;
        validate_timestamp(&raw.created_at)?;
        validate_timestamp(&raw.expires_at)?;
        if raw.audit_trail.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding("audit_trail"));
        }
        Ok(Self {
            id: raw.id,
            schema_version: raw.schema_version,
            scope: raw.scope,
            observation_ids: raw.observation_ids,
            source: raw.source,
            target: raw.target,
            reason: raw.reason,
            created_by: raw.created_by,
            created_at: raw.created_at,
            expires_at: raw.expires_at,
            approval_ref: raw.approval_ref,
            audit_trail: raw.audit_trail,
        })
    }
}

impl<'de> Deserialize<'de> for EvidenceWaiver {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(EvidenceWaiverRaw::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustedProvenance {
    pub source: ProvenanceSourceKind,
    pub assigned_by: String,
    pub execution_id: String,
    pub tool_call_id: Option<String>,
}

impl TrustedProvenance {
    pub(crate) fn planr_observed_execution(
        execution_id: impl Into<String>,
    ) -> Result<Self, EvidenceDomainError> {
        let execution_id = execution_id.into();
        if execution_id.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding("execution_id"));
        }
        Ok(Self {
            source: ProvenanceSourceKind::PlanrObservedExecution,
            assigned_by: "planr".to_string(),
            execution_id,
            tool_call_id: None,
        })
    }

    fn validate(&self) -> Result<(), EvidenceDomainError> {
        if self.assigned_by != "planr" {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "provenance.assigned_by",
            ));
        }
        EvidenceId::parse(self.execution_id.clone())?;
        if let Some(tool_call_id) = &self.tool_call_id {
            EvidenceId::parse(tool_call_id.clone())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceBinding {
    pub revision: String,
    pub tree_digest: Sha256Digest,
    pub dirty: bool,
}

impl SourceBinding {
    pub(crate) fn validate(&self) -> Result<(), EvidenceDomainError> {
        if self.revision.chars().count() < 7 {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "source.revision",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetBinding {
    pub kind: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub uri: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub digest: Option<Sha256Digest>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub deployment_id: Option<EvidenceId>,
}

impl TargetBinding {
    pub(crate) fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.kind, "target.kind")?;
        let has_identity = self.uri.as_deref().is_some_and(|uri| !uri.is_empty())
            || self.digest.is_some()
            || self.deployment_id.is_some();
        if has_identity {
            Ok(())
        } else {
            Err(EvidenceDomainError::MissingTrustedBinding(
                "target.uri|digest|deployment_id",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentBinding {
    pub kind: String,
    pub id: EvidenceId,
    pub digest: Sha256Digest,
}

impl EnvironmentBinding {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.kind, "environment.kind")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VantagePoint {
    pub kind: String,
    pub identity: String,
}

impl VantagePoint {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.kind, "vantage_point.kind")?;
        require_non_empty(&self.identity, "vantage_point.identity")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityBinding {
    pub manifest_id: EvidenceId,
    pub manifest_digest: Sha256Digest,
    pub instance_id: EvidenceId,
    pub instance_digest: Sha256Digest,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationResult {
    pub requirement_id: EvidenceId,
    #[serde(rename = "type")]
    pub observation_type: NamespacedIdentifier,
    pub outcome: AttemptStatus,
    pub predicate: Map<String, Value>,
    pub actual: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ArtifactRef {
    pub id: EvidenceId,
    pub kind: String,
    pub digest: Sha256Digest,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub uri: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ArtifactRef {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.kind, "artifacts[].kind")?;
        if let Some(uri) = &self.uri {
            require_non_empty(uri, "artifacts[].uri")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct RawResultRef {
    pub kind: String,
    pub digest: Sha256Digest,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub artifact_id: Option<EvidenceId>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl RawResultRef {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.kind, "raw_result.kind")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureDisclosure {
    pub fixtures_used: bool,
    pub mocks_used: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_list_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub fixture_refs: Option<Vec<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_list_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub mock_refs: Option<Vec<String>>,
}

impl FixtureDisclosure {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        validate_optional_string_list(
            self.fixture_refs.as_deref(),
            "fixture_disclosure.fixture_refs",
        )?;
        validate_optional_string_list(self.mock_refs.as_deref(), "fixture_disclosure.mock_refs")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionState {
    pub network: String,
    pub filesystem: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub environment: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_string_without_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub secrets: Option<String>,
}

impl PermissionState {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.network, "permissions.network")?;
        require_non_empty(&self.filesystem, "permissions.filesystem")?;
        if let Some(environment) = &self.environment {
            require_non_empty(environment, "permissions.environment")?;
        }
        if let Some(secrets) = &self.secrets {
            require_non_empty(secrets, "permissions.secrets")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SandboxState {
    pub mode: String,
    pub limits: SandboxLimits,
}

impl SandboxState {
    fn validate(&self) -> Result<(), EvidenceDomainError> {
        require_non_empty(&self.mode, "sandbox.mode")?;
        if self.limits.timeout_ms < 1 {
            return Err(EvidenceDomainError::InvalidTrustedBinding(
                "sandbox.limits.timeout_ms",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SandboxLimits {
    pub timeout_ms: u64,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceReceipt {
    id: EvidenceId,
    schema_version: SchemaVersion,
    criterion_id: EvidenceId,
    obligation_id: EvidenceId,
    receipt_status: ReceiptStatus,
    source: SourceBinding,
    target: TargetBinding,
    environment: EnvironmentBinding,
    vantage_point: VantagePoint,
    capability: CapabilityBinding,
    provenance: TrustedProvenance,
    observations: Vec<ObservationResult>,
    attempt_ids: Vec<EvidenceId>,
    retry_history: Vec<Map<String, Value>>,
    artifacts: Vec<ArtifactRef>,
    raw_result: RawResultRef,
    config_digest: Sha256Digest,
    fixture_disclosure: FixtureDisclosure,
    permissions: PermissionState,
    sandbox: SandboxState,
    proof_gaps: Vec<GapReason>,
    started_at: String,
    ended_at: String,
    receipt_digest: Sha256Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceReceiptRaw {
    id: EvidenceId,
    schema_version: SchemaVersion,
    criterion_id: EvidenceId,
    obligation_id: EvidenceId,
    receipt_status: ReceiptStatus,
    source: SourceBinding,
    target: TargetBinding,
    environment: EnvironmentBinding,
    vantage_point: VantagePoint,
    capability: CapabilityBinding,
    provenance: TrustedProvenance,
    observations: Vec<ObservationResult>,
    attempt_ids: Vec<EvidenceId>,
    retry_history: Vec<Map<String, Value>>,
    artifacts: Vec<ArtifactRef>,
    raw_result: RawResultRef,
    config_digest: Sha256Digest,
    fixture_disclosure: FixtureDisclosure,
    permissions: PermissionState,
    sandbox: SandboxState,
    proof_gaps: Vec<String>,
    started_at: String,
    ended_at: String,
    receipt_digest: Sha256Digest,
}

impl EvidenceReceipt {
    pub(crate) fn from_trusted_value(value: Value) -> Result<Self, EvidenceDomainError> {
        let raw: EvidenceReceiptRaw = serde_json::from_value(value.clone())
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
        let receipt = Self::try_from(raw)?;
        let digest = sha256_json_digest_without_top_level_field(&value, "receipt_digest")
            .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
        if receipt.receipt_digest.0 != digest {
            return Err(EvidenceDomainError::InvalidTrustedBinding("receipt_digest"));
        }
        Ok(receipt)
    }

    pub(crate) fn receipt_digest(&self) -> &Sha256Digest {
        &self.receipt_digest
    }

    pub(crate) fn obligation_id(&self) -> &EvidenceId {
        &self.obligation_id
    }

    pub(crate) fn source(&self) -> &SourceBinding {
        &self.source
    }

    pub(crate) fn target(&self) -> &TargetBinding {
        &self.target
    }

    pub(crate) fn environment(&self) -> &EnvironmentBinding {
        &self.environment
    }

    pub(crate) fn capability(&self) -> &CapabilityBinding {
        &self.capability
    }

    pub(crate) fn config_digest(&self) -> &Sha256Digest {
        &self.config_digest
    }

    pub(crate) fn attempt_ids(&self) -> &[EvidenceId] {
        &self.attempt_ids
    }

    pub(crate) fn observations(&self) -> &[ObservationResult] {
        &self.observations
    }

    pub(crate) fn raw_result(&self) -> &RawResultRef {
        &self.raw_result
    }

    pub(crate) fn provenance(&self) -> &TrustedProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TrustedPolicySource {
    Repository,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustedReceiptBinding {
    pub source: SourceBinding,
    pub target: TargetBinding,
    pub environment: EnvironmentBinding,
    pub capability: CapabilityBinding,
    pub config_digest: Sha256Digest,
    pub policy_digest: Sha256Digest,
    pub policy_source: TrustedPolicySource,
}

impl TrustedReceiptBinding {
    pub(crate) fn from_receipt(
        receipt: &EvidenceReceipt,
        policy_digest: Sha256Digest,
        policy_source: TrustedPolicySource,
    ) -> Self {
        Self {
            source: receipt.source().clone(),
            target: receipt.target().clone(),
            environment: receipt.environment().clone(),
            capability: receipt.capability().clone(),
            config_digest: receipt.config_digest().clone(),
            policy_digest,
            policy_source,
        }
    }

    pub(crate) fn validate_receipt_exact(
        &self,
        receipt: &EvidenceReceipt,
    ) -> Result<(), EvidenceDomainError> {
        if &self.source != receipt.source() {
            return Err(EvidenceDomainError::InvalidTrustedBinding("source"));
        }
        if &self.target != receipt.target() {
            return Err(EvidenceDomainError::InvalidTrustedBinding("target"));
        }
        if &self.environment != receipt.environment() {
            return Err(EvidenceDomainError::InvalidTrustedBinding("environment"));
        }
        if &self.capability != receipt.capability() {
            return Err(EvidenceDomainError::InvalidTrustedBinding("capability"));
        }
        if &self.config_digest != receipt.config_digest() {
            return Err(EvidenceDomainError::InvalidTrustedBinding("config_digest"));
        }
        Ok(())
    }
}

impl TryFrom<EvidenceReceiptRaw> for EvidenceReceipt {
    type Error = EvidenceDomainError;

    fn try_from(raw: EvidenceReceiptRaw) -> Result<Self, Self::Error> {
        if raw.receipt_status != ReceiptStatus::Trusted {
            return Err(EvidenceDomainError::InvalidTrustedBinding("receipt_status"));
        }
        if raw.observations.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding("observations"));
        }
        if raw.attempt_ids.is_empty() {
            return Err(EvidenceDomainError::MissingTrustedBinding("attempt_ids"));
        }
        validate_timestamp(&raw.started_at)?;
        validate_timestamp(&raw.ended_at)?;
        raw.source.validate()?;
        raw.target.validate()?;
        raw.environment.validate()?;
        raw.vantage_point.validate()?;
        raw.provenance.validate()?;
        for artifact in &raw.artifacts {
            artifact.validate()?;
        }
        raw.raw_result.validate()?;
        raw.fixture_disclosure.validate()?;
        raw.permissions.validate()?;
        raw.sandbox.validate()?;
        Ok(Self {
            id: raw.id,
            schema_version: raw.schema_version,
            criterion_id: raw.criterion_id,
            obligation_id: raw.obligation_id,
            receipt_status: raw.receipt_status,
            source: raw.source,
            target: raw.target,
            environment: raw.environment,
            vantage_point: raw.vantage_point,
            capability: raw.capability,
            provenance: raw.provenance,
            observations: raw.observations,
            attempt_ids: raw.attempt_ids,
            retry_history: raw.retry_history,
            artifacts: raw.artifacts,
            raw_result: raw.raw_result,
            config_digest: raw.config_digest,
            fixture_disclosure: raw.fixture_disclosure,
            permissions: raw.permissions,
            sandbox: raw.sandbox,
            proof_gaps: raw
                .proof_gaps
                .iter()
                .map(|gap| GapReason::canonicalize(gap.as_str()))
                .collect(),
            started_at: raw.started_at,
            ended_at: raw.ended_at,
            receipt_digest: raw.receipt_digest,
        })
    }
}

pub(crate) struct TrustedReceiptInput {
    pub id: EvidenceId,
    pub criterion_id: EvidenceId,
    pub obligation_id: EvidenceId,
    pub source: SourceBinding,
    pub target: TargetBinding,
    pub environment: EnvironmentBinding,
    pub vantage_point: VantagePoint,
    pub capability: CapabilityBinding,
    pub provenance: TrustedProvenance,
    pub observations: Vec<ObservationResult>,
    pub attempt_ids: Vec<EvidenceId>,
    pub retry_history: Vec<Map<String, Value>>,
    pub artifacts: Vec<ArtifactRef>,
    pub raw_result: RawResultRef,
    pub config_digest: Sha256Digest,
    pub fixture_disclosure: FixtureDisclosure,
    pub permissions: PermissionState,
    pub sandbox: SandboxState,
    pub proof_gaps: Vec<GapReason>,
    pub started_at: String,
    pub ended_at: String,
}

pub(crate) fn build_trusted_receipt(
    input: TrustedReceiptInput,
) -> Result<EvidenceReceipt, EvidenceDomainError> {
    if input.observations.is_empty() {
        return Err(EvidenceDomainError::MissingTrustedBinding("observations"));
    }
    if input.attempt_ids.is_empty() {
        return Err(EvidenceDomainError::MissingTrustedBinding("attempt_ids"));
    }
    validate_timestamp(&input.started_at)?;
    validate_timestamp(&input.ended_at)?;
    input.source.validate()?;
    input.target.validate()?;
    input.environment.validate()?;
    input.vantage_point.validate()?;
    input.provenance.validate()?;
    for artifact in &input.artifacts {
        artifact.validate()?;
    }
    input.raw_result.validate()?;
    input.fixture_disclosure.validate()?;
    input.permissions.validate()?;
    input.sandbox.validate()?;

    let value = serde_json::to_value(EvidenceReceipt {
        id: input.id.clone(),
        schema_version: SchemaVersion::v1(),
        criterion_id: input.criterion_id.clone(),
        obligation_id: input.obligation_id.clone(),
        receipt_status: ReceiptStatus::Trusted,
        source: input.source.clone(),
        target: input.target.clone(),
        environment: input.environment.clone(),
        vantage_point: input.vantage_point.clone(),
        capability: input.capability.clone(),
        provenance: input.provenance.clone(),
        observations: input.observations.clone(),
        attempt_ids: input.attempt_ids.clone(),
        retry_history: input.retry_history.clone(),
        artifacts: input.artifacts.clone(),
        raw_result: input.raw_result.clone(),
        config_digest: input.config_digest.clone(),
        fixture_disclosure: input.fixture_disclosure.clone(),
        permissions: input.permissions.clone(),
        sandbox: input.sandbox.clone(),
        proof_gaps: input.proof_gaps.clone(),
        started_at: input.started_at.clone(),
        ended_at: input.ended_at.clone(),
        receipt_digest: Sha256Digest::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )?,
    })
    .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;
    let digest = sha256_json_digest_without_top_level_field(&value, "receipt_digest")
        .map_err(|err| EvidenceDomainError::Digest(err.to_string()))?;

    Ok(EvidenceReceipt {
        id: input.id,
        schema_version: SchemaVersion::v1(),
        criterion_id: input.criterion_id,
        obligation_id: input.obligation_id,
        receipt_status: ReceiptStatus::Trusted,
        source: input.source,
        target: input.target,
        environment: input.environment,
        vantage_point: input.vantage_point,
        capability: input.capability,
        provenance: input.provenance,
        observations: input.observations,
        attempt_ids: input.attempt_ids,
        retry_history: input.retry_history,
        artifacts: input.artifacts,
        raw_result: input.raw_result,
        config_digest: input.config_digest,
        fixture_disclosure: input.fixture_disclosure,
        permissions: input.permissions,
        sandbox: input.sandbox,
        proof_gaps: input.proof_gaps,
        started_at: input.started_at,
        ended_at: input.ended_at,
        receipt_digest: Sha256Digest::parse(digest)?,
    })
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
}

fn is_valid_namespaced_identifier(value: &str) -> bool {
    if let Some(tail) = value
        .strip_prefix("planr.")
        .or_else(|| value.strip_prefix("mcp."))
        .or_else(|| value.strip_prefix("host."))
        .or_else(|| value.strip_prefix("project."))
    {
        return is_valid_namespace_tail(tail);
    }
    value.match_indices('.').any(|(index, _)| {
        let (prefix, tail_with_dot) = value.split_at(index);
        is_valid_reverse_domain_prefix(prefix) && is_valid_namespace_tail(&tail_with_dot[1..])
    })
}

fn is_valid_reverse_domain_prefix(prefix: &str) -> bool {
    let segments = prefix.split('.').collect::<Vec<_>>();
    segments.len() >= 2
        && segments.first().is_some_and(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
        && segments.iter().skip(1).all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn is_valid_namespace_tail(tail: &str) -> bool {
    let mut chars = tail.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
        })
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), EvidenceDomainError> {
    if value.is_empty() {
        Err(EvidenceDomainError::InvalidTrustedBinding(field))
    } else {
        Ok(())
    }
}

fn validate_optional_string_list(
    values: Option<&[String]>,
    field: &'static str,
) -> Result<(), EvidenceDomainError> {
    for value in values.into_iter().flatten() {
        require_non_empty(value, field)?;
    }
    Ok(())
}

fn validate_string_list(values: &[String], field: &'static str) -> Result<(), EvidenceDomainError> {
    for value in values {
        require_non_empty(value, field)?;
    }
    Ok(())
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

fn deserialize_optional_string_list_without_null<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Null => Err(de::Error::custom(
            "optional string list field must be omitted or an array, not null",
        )),
        value => serde_json::from_value::<Vec<String>>(value)
            .map(Some)
            .map_err(de::Error::custom),
    }
}

pub(crate) fn validate_timestamp(value: &str) -> Result<(), EvidenceDomainError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|_| ())
        .map_err(|_| EvidenceDomainError::InvalidTimestamp(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refresh_receipt_digest(value: &mut Value) {
        let digest = sha256_json_digest_without_top_level_field(value, "receipt_digest").unwrap();
        value["receipt_digest"] = json!(digest);
    }

    #[test]
    fn exact_status_parsing_rejects_case_and_unknown_values() {
        assert_eq!(
            "passed".parse::<AttemptStatus>().unwrap(),
            AttemptStatus::Passed
        );
        assert!("Passed".parse::<AttemptStatus>().is_err());
        assert!("complete".parse::<CoverageStatus>().is_err());
        assert_eq!(ReceiptStatus::Trusted.as_str(), "trusted");
    }

    #[test]
    fn namespaced_identifiers_allow_reserved_and_reverse_domain_forms() {
        assert!(NamespacedIdentifier::parse("planr.api.http.response").is_ok());
        assert!(NamespacedIdentifier::parse("planr.api_http.response").is_ok());
        assert!(NamespacedIdentifier::parse("planr.api._foo").is_ok());
        assert!(NamespacedIdentifier::parse("com.example.queue.job.processed").is_ok());
        assert!(NamespacedIdentifier::parse("com.-example.queue").is_ok());
        assert!(NamespacedIdentifier::parse("api.http").is_err());
        assert!(NamespacedIdentifier::parse("Planr.api").is_err());
        assert!(NamespacedIdentifier::parse("planr._foo").is_err());
        assert!(NamespacedIdentifier::parse("com_example.queue.job").is_err());
    }

    #[test]
    fn namespaced_identifier_parser_matches_frozen_schema_corpus() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let validator = jsonschema::draft202012::options()
            .build(&schema["$defs"]["NamespaceName"])
            .expect("NamespaceName schema compiles");

        for value in [
            "planr.api.http.response",
            "planr.api_http.response",
            "planr.api._foo",
            "mcp.server.tool",
            "host.chrome.tab",
            "project.custom-check",
            "com.example.queue.job.processed",
            "com.-example.queue",
            "a.b.c",
            "api.http",
            "Planr.api",
            "planr._foo",
            "planr.",
            "com_example.queue.job",
            "com.example._queue",
            "com..example.queue",
            "com.Example.queue",
            "com.example",
        ] {
            let parser_accepts = NamespacedIdentifier::parse(value).is_ok();
            let schema_accepts = validator.is_valid(&json!(value));
            assert_eq!(
                parser_accepts, schema_accepts,
                "NamespaceName parser/schema mismatch for {value}"
            );
        }
    }

    #[test]
    fn source_kind_matches_frozen_contract_values() {
        for value in ["agent", "adapter", "host", "mcp", "artifact_import", "user"] {
            assert_eq!(value.parse::<SourceKind>().unwrap().as_str(), value);
        }
        assert!("import".parse::<SourceKind>().is_err());
    }

    #[test]
    fn observation_schema_bindings_round_trip_fixtures_and_reject_incomplete_values() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let obligation: ProofObligation = serde_json::from_str(
            &std::fs::read_to_string(
                root.join("docs/contracts/fixtures/evidence/v1/examples/proof-obligation.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            obligation.observations[0]
                .payload_schema
                .as_ref()
                .unwrap()
                .schema_ref,
            "planr.api.http.response@v1"
        );

        let manifest: VerificationCapabilityManifest = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            manifest.supported_observations[0].observation_type.as_str(),
            "planr.api.http.response"
        );

        let instance: VerificationCapabilityInstance = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-instance.json",
            ))
            .unwrap(),
        )
        .unwrap();
        instance.observed_payload_contract.validate().unwrap();

        let mut invalid_manifest = serde_json::to_value(&manifest).unwrap();
        invalid_manifest["supported_observations"][0]
            .as_object_mut()
            .unwrap()
            .remove("schema_digest");
        assert!(
            serde_json::from_value::<VerificationCapabilityManifest>(invalid_manifest).is_err()
        );

        let mut empty_schema_ref = serde_json::to_value(&manifest).unwrap();
        empty_schema_ref["supported_observations"][0]["schema_ref"] = json!("");
        assert!(
            serde_json::from_value::<VerificationCapabilityManifest>(empty_schema_ref).is_err()
        );

        let mut empty_supported_observations = serde_json::to_value(&manifest).unwrap();
        empty_supported_observations["supported_observations"] = json!([]);
        assert!(
            serde_json::from_value::<VerificationCapabilityManifest>(empty_supported_observations)
                .is_err()
        );

        let mut invalid_instance = serde_json::to_value(&instance).unwrap();
        invalid_instance["observed_payload_contract"]["observation_types"] = json!(["planr._bad"]);
        assert!(
            serde_json::from_value::<VerificationCapabilityInstance>(invalid_instance).is_err()
        );

        let mut empty_observation_types = serde_json::to_value(&instance).unwrap();
        empty_observation_types["observed_payload_contract"]["observation_types"] = json!([]);
        assert!(
            serde_json::from_value::<VerificationCapabilityInstance>(empty_observation_types)
                .is_err()
        );

        let mut empty_observed_schema_ref = serde_json::to_value(&instance).unwrap();
        empty_observed_schema_ref["observed_payload_contract"]["schema_ref"] = json!("");
        assert!(
            serde_json::from_value::<VerificationCapabilityInstance>(empty_observed_schema_ref)
                .is_err()
        );
    }

    #[test]
    fn runtime_targets_preserve_schema_allowed_adapter_extensions() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let mut extended_manifest = manifest.clone();
        extended_manifest["runtime_targets"][0]["adapter_namespace"] = json!("com.example.runtime");
        extended_manifest["runtime_targets"][0]["adapter_limits"] = json!({
            "max_tabs": 3,
            "headless": false
        });

        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let validator = jsonschema::draft202012::options()
            .build(&schema)
            .expect("Evidence contract schema compiles");
        assert!(
            validator.is_valid(&extended_manifest),
            "extended RuntimeTarget sample must remain schema-valid"
        );

        let parsed: VerificationCapabilityManifest =
            serde_json::from_value(extended_manifest.clone()).unwrap();
        assert_eq!(
            parsed.runtime_targets[0]
                .extensions
                .get("adapter_namespace")
                .and_then(Value::as_str),
            Some("com.example.runtime")
        );
        assert_eq!(
            parsed.runtime_targets[0].extensions.get("adapter_limits"),
            Some(&json!({"max_tabs": 3, "headless": false}))
        );

        let round_tripped = serde_json::to_value(parsed).unwrap();
        assert_eq!(
            round_tripped["runtime_targets"][0]["adapter_namespace"],
            extended_manifest["runtime_targets"][0]["adapter_namespace"]
        );
        assert_eq!(
            round_tripped["runtime_targets"][0]["adapter_limits"],
            extended_manifest["runtime_targets"][0]["adapter_limits"]
        );

        let mut invalid_kind = extended_manifest.clone();
        invalid_kind["runtime_targets"][0]["kind"] = json!("");
        assert!(
            !validator.is_valid(&invalid_kind),
            "empty RuntimeTarget.kind must remain schema-invalid"
        );
        assert!(
            serde_json::from_value::<VerificationCapabilityManifest>(invalid_kind).is_err(),
            "empty RuntimeTarget.kind must remain model-invalid"
        );

        let mut invalid_id = extended_manifest;
        invalid_id["runtime_targets"][0]["id"] = json!("bad runtime id");
        assert!(
            !validator.is_valid(&invalid_id),
            "invalid RuntimeTarget.id must remain schema-invalid"
        );
        assert!(
            serde_json::from_value::<VerificationCapabilityManifest>(invalid_id).is_err(),
            "invalid RuntimeTarget.id must remain model-invalid"
        );
    }

    #[test]
    fn capability_manifests_reject_frozen_schema_invalid_public_boundary_values() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-manifest.json",
            ))
            .unwrap(),
        )
        .unwrap();

        for (label, path, replacement) in [
            ("adapter_kind", &["adapter_kind"][..], json!("bogus")),
            ("version", &["version"][..], json!("")),
            (
                "supported_surfaces",
                &["supported_surfaces"][..],
                json!([""]),
            ),
            (
                "supported_interactions",
                &["supported_interactions"][..],
                json!([""]),
            ),
            (
                "supported_artifacts",
                &["supported_artifacts"][..],
                json!([""]),
            ),
            ("runtime_targets", &["runtime_targets"][..], json!([])),
            (
                "runtime_targets.kind",
                &["runtime_targets", "0", "kind"][..],
                json!(""),
            ),
            ("permissions", &["permissions"][..], Value::Null),
            ("costs", &["costs"][..], Value::Null),
            ("determinism", &["determinism"][..], json!("")),
            ("repeatability", &["repeatability"][..], json!("")),
            ("independence", &["independence"][..], json!("")),
            ("blind_spots", &["blind_spots"][..], json!([""])),
            (
                "availability_probe",
                &["availability_probe"][..],
                Value::Null,
            ),
            (
                "availability_probe.kind",
                &["availability_probe", "kind"][..],
                json!("host"),
            ),
            (
                "availability_probe.execution.executable",
                &["availability_probe", "execution", "executable"][..],
                json!(""),
            ),
            (
                "availability_probe.execution.timeout_ms",
                &["availability_probe", "execution", "timeout_ms"][..],
                json!(0),
            ),
            (
                "availability_probe.execution.working_directory",
                &["availability_probe", "execution", "working_directory"][..],
                json!(""),
            ),
        ] {
            let mut invalid = manifest.clone();
            assign_nested(&mut invalid, path, replacement);
            assert!(
                serde_json::from_value::<VerificationCapabilityManifest>(invalid).is_err(),
                "{label} should be rejected by manifest deserialization"
            );
        }
    }

    #[test]
    fn capability_instances_reject_frozen_schema_invalid_boundary_values() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let instance: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-instance.json",
            ))
            .unwrap(),
        )
        .unwrap();

        for (label, path, replacement) in [
            ("host", &["host"][..], json!("")),
            ("surface", &["surface"][..], json!("")),
            ("host_version", &["host_version"][..], json!("")),
            ("adapter_version", &["adapter_version"][..], json!("")),
            ("environment", &["environment"][..], Value::Null),
            ("environment.kind", &["environment", "kind"][..], json!("")),
            ("permissions", &["permissions"][..], Value::Null),
            (
                "permissions.network",
                &["permissions", "network"][..],
                json!(""),
            ),
            (
                "permissions.filesystem",
                &["permissions", "filesystem"][..],
                json!(""),
            ),
            ("limitations.item", &["limitations", "0"][..], json!("")),
            ("captured_at", &["captured_at"][..], json!("notTdateZ")),
        ] {
            let mut invalid = instance.clone();
            assign_nested(&mut invalid, path, replacement);
            assert!(
                serde_json::from_value::<VerificationCapabilityInstance>(invalid).is_err(),
                "{label} should be rejected by instance deserialization"
            );
        }
    }

    #[test]
    fn capability_instances_reject_unknown_statuses_at_deserialization_boundary() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let instance: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-instance.json",
            ))
            .unwrap(),
        )
        .unwrap();

        let mut invalid_availability = instance.clone();
        invalid_availability["availability"]["status"] = json!("AVAILABLE");
        assert!(
            serde_json::from_value::<VerificationCapabilityInstance>(invalid_availability).is_err()
        );

        let mut invalid_probe = instance.clone();
        invalid_probe["probe_result"]["outcome"] = json!("complete");
        assert!(serde_json::from_value::<VerificationCapabilityInstance>(invalid_probe).is_err());

        let mut empty_checks = instance;
        empty_checks["probe_result"]["checks"] = json!([]);
        assert!(serde_json::from_value::<VerificationCapabilityInstance>(empty_checks).is_err());
    }

    #[test]
    fn trusted_receipts_are_built_with_private_planr_provenance_and_digest() {
        let input = trusted_receipt_input();
        let receipt = build_trusted_receipt(input).unwrap();

        assert_eq!(receipt.receipt_status, ReceiptStatus::Trusted);
        assert_eq!(receipt.provenance.assigned_by, "planr");
        assert!(receipt.receipt_digest.0.starts_with("sha256:"));
        assert_ne!(
            receipt.receipt_digest.0,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_validates_against_frozen_schema(&serde_json::to_value(receipt).unwrap());
    }

    #[test]
    fn trusted_receipt_constructor_rejects_invalid_trust_bindings() {
        let mut missing_target = trusted_receipt_input();
        missing_target.target.uri = None;
        assert!(build_trusted_receipt(missing_target).is_err());

        let mut unicode_short_revision = trusted_receipt_input();
        unicode_short_revision.source.revision = "\u{1F600}\u{1F600}".to_string();
        assert!(build_trusted_receipt(unicode_short_revision).is_err());

        let mut unicode_min_revision = trusted_receipt_input();
        unicode_min_revision.source.revision =
            "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}".to_string();
        assert_validates_against_frozen_schema(
            &serde_json::to_value(build_trusted_receipt(unicode_min_revision).unwrap()).unwrap(),
        );

        let mut invalid_artifact = trusted_receipt_input();
        invalid_artifact.artifacts.push(ArtifactRef {
            id: EvidenceId::parse("artifact-test").unwrap(),
            kind: "stdout".to_string(),
            digest: Sha256Digest::parse(
                "sha256:8888888888888888888888888888888888888888888888888888888888888888",
            )
            .unwrap(),
            uri: Some(String::new()),
            extra: Map::new(),
        });
        assert!(build_trusted_receipt(invalid_artifact).is_err());

        let mut invalid_timestamp = trusted_receipt_input();
        invalid_timestamp.ended_at = "xTZ".to_string();
        assert!(build_trusted_receipt(invalid_timestamp).is_err());

        let mut invalid_fixture_ref = trusted_receipt_input();
        invalid_fixture_ref.fixture_disclosure.fixture_refs = Some(vec![String::new()]);
        assert!(build_trusted_receipt(invalid_fixture_ref).is_err());

        let mut invalid_mock_ref = trusted_receipt_input();
        invalid_mock_ref.fixture_disclosure.mock_refs = Some(vec![String::new()]);
        assert!(build_trusted_receipt(invalid_mock_ref).is_err());

        let mut invalid_retry = trusted_receipt_input();
        invalid_retry.retry_history = vec![Map::from_iter([(
            "attempt".to_string(),
            Value::String("retry-1".to_string()),
        )])];
        assert!(build_trusted_receipt(invalid_retry).is_ok());

        let mut offset_timestamp = trusted_receipt_input();
        offset_timestamp.ended_at = "2026-07-28T14:00:01+02:00".to_string();
        assert!(build_trusted_receipt(offset_timestamp).is_ok());
    }

    #[test]
    fn trusted_receipt_constructor_digest_matches_fixture_vector() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture_value: Value = serde_json::from_str(
            &std::fs::read_to_string(
                root.join("docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let fixture = EvidenceReceipt::from_trusted_value(fixture_value.clone()).unwrap();
        let expected_digest = fixture.receipt_digest.0.clone();
        let input = TrustedReceiptInput {
            id: fixture.id,
            criterion_id: fixture.criterion_id,
            obligation_id: fixture.obligation_id,
            source: fixture.source,
            target: fixture.target,
            environment: fixture.environment,
            vantage_point: fixture.vantage_point,
            capability: fixture.capability,
            provenance: fixture.provenance,
            observations: fixture.observations,
            attempt_ids: fixture.attempt_ids,
            retry_history: fixture.retry_history,
            artifacts: fixture.artifacts,
            raw_result: fixture.raw_result,
            config_digest: fixture.config_digest,
            fixture_disclosure: fixture.fixture_disclosure,
            permissions: fixture.permissions,
            sandbox: fixture.sandbox,
            proof_gaps: fixture.proof_gaps,
            started_at: fixture.started_at,
            ended_at: fixture.ended_at,
        };
        let receipt = build_trusted_receipt(input).unwrap();
        assert_eq!(receipt.receipt_digest.0, expected_digest);
        assert_validates_against_frozen_schema(&serde_json::to_value(receipt).unwrap());

        let mut forged = fixture_value;
        forged["receipt_digest"] =
            json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
        assert!(EvidenceReceipt::from_trusted_value(forged).is_err());
    }

    #[test]
    fn trusted_receipt_rehydration_rejects_frozen_schema_invalid_json() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture_value: Value = serde_json::from_str(
            &std::fs::read_to_string(
                root.join("docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json"),
            )
            .unwrap(),
        )
        .unwrap();

        let mut invalid_retry = fixture_value.clone();
        invalid_retry["retry_history"] = json!([null]);
        refresh_receipt_digest(&mut invalid_retry);
        assert!(EvidenceReceipt::from_trusted_value(invalid_retry).is_err());

        let mut legacy_gap = fixture_value.clone();
        legacy_gap["proof_gaps"] = json!(["capability_unavailable", "not_a_gap_reason"]);
        refresh_receipt_digest(&mut legacy_gap);
        let rehydrated = EvidenceReceipt::from_trusted_value(legacy_gap).unwrap();
        assert_eq!(
            rehydrated.proof_gaps,
            vec![GapReason::MissingCapability, GapReason::VerifierFailed]
        );

        let mut invalid_timestamp = fixture_value.clone();
        invalid_timestamp["ended_at"] = json!("notTdateZ");
        refresh_receipt_digest(&mut invalid_timestamp);
        assert!(EvidenceReceipt::from_trusted_value(invalid_timestamp).is_err());

        let mut unicode_short_revision = fixture_value.clone();
        unicode_short_revision["source"]["revision"] = json!("\u{1F600}\u{1F600}");
        refresh_receipt_digest(&mut unicode_short_revision);
        assert_rejected_by_frozen_schema(&unicode_short_revision);
        assert!(EvidenceReceipt::from_trusted_value(unicode_short_revision).is_err());

        let mut invalid_fixture_ref = fixture_value.clone();
        invalid_fixture_ref["fixture_disclosure"]["fixture_refs"] = json!([""]);
        refresh_receipt_digest(&mut invalid_fixture_ref);
        assert!(EvidenceReceipt::from_trusted_value(invalid_fixture_ref).is_err());

        let mut invalid_mock_ref = fixture_value.clone();
        invalid_mock_ref["fixture_disclosure"]["mock_refs"] = json!([""]);
        refresh_receipt_digest(&mut invalid_mock_ref);
        assert!(EvidenceReceipt::from_trusted_value(invalid_mock_ref).is_err());

        let mut null_fixture_ref = fixture_value.clone();
        null_fixture_ref["fixture_disclosure"]["fixture_refs"] = Value::Null;
        refresh_receipt_digest(&mut null_fixture_ref);
        assert!(EvidenceReceipt::from_trusted_value(null_fixture_ref).is_err());

        let mut null_mock_ref = fixture_value.clone();
        null_mock_ref["fixture_disclosure"]["mock_refs"] = Value::Null;
        refresh_receipt_digest(&mut null_mock_ref);
        assert!(EvidenceReceipt::from_trusted_value(null_mock_ref).is_err());

        let mut null_artifact_uri = fixture_value;
        null_artifact_uri["artifacts"][0]["uri"] = Value::Null;
        refresh_receipt_digest(&mut null_artifact_uri);
        assert!(EvidenceReceipt::from_trusted_value(null_artifact_uri).is_err());
    }

    #[test]
    fn trusted_rehydration_rejects_schema_forbidden_explicit_null_optionals() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture_value: Value = serde_json::from_str(
            &std::fs::read_to_string(
                root.join("docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json"),
            )
            .unwrap(),
        )
        .unwrap();

        for path in [
            &["target", "uri"][..],
            &["target", "digest"][..],
            &["target", "deployment_id"][..],
            &["raw_result", "artifact_id"][..],
            &["permissions", "environment"][..],
            &["permissions", "secrets"][..],
        ] {
            let mut invalid = fixture_value.clone();
            assign_nested(&mut invalid, path, Value::Null);
            refresh_receipt_digest(&mut invalid);
            assert!(
                EvidenceReceipt::from_trusted_value(invalid).is_err(),
                "{}:null should be rejected during trusted receipt rehydration",
                path.join(".")
            );
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut obligation: Value = serde_json::from_str(
            &std::fs::read_to_string(
                root.join("docs/contracts/fixtures/evidence/v1/examples/proof-obligation.json"),
            )
            .unwrap(),
        )
        .unwrap();
        for field in [
            "payload_schema",
            "state_transitions",
            "persistence",
            "negative_assertions",
            "freshness_policy",
            "assurance_policy",
        ] {
            let mut invalid = obligation.clone();
            invalid["observations"][0][field] = Value::Null;
            assert!(
                serde_json::from_value::<ProofObligation>(invalid).is_err(),
                "observations[].{field}:null should be rejected"
            );
        }
        obligation["item_id"] = Value::Null;
        obligation["supersedes"] = Value::Null;
        assert!(
            serde_json::from_value::<ProofObligation>(obligation).is_ok(),
            "schema-nullable ProofObligation bindings should remain accepted"
        );

        let mut instance: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-instance.json",
            ))
            .unwrap(),
        )
        .unwrap();
        instance["availability"]["reason"] = Value::Null;
        assert!(serde_json::from_value::<VerificationCapabilityInstance>(instance).is_err());

        let mut instance: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/examples/verification-capability-instance.json",
            ))
            .unwrap(),
        )
        .unwrap();
        instance["probe_result"]["checks"][0]["detail"] = Value::Null;
        assert!(serde_json::from_value::<VerificationCapabilityInstance>(instance).is_err());
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

    fn trusted_receipt_input() -> TrustedReceiptInput {
        TrustedReceiptInput {
            id: EvidenceId::parse("erec-test").unwrap(),
            criterion_id: EvidenceId::parse("criterion-test").unwrap(),
            obligation_id: EvidenceId::parse("pob-test").unwrap(),
            source: SourceBinding {
                revision: "0123456789abcdef0123456789abcdef01234567".to_string(),
                tree_digest: Sha256Digest::parse(
                    "sha256:3333333333333333333333333333333333333333333333333333333333333333",
                )
                .unwrap(),
                dirty: false,
            },
            target: TargetBinding {
                kind: "local".to_string(),
                uri: Some("http://127.0.0.1".to_string()),
                digest: None,
                deployment_id: None,
            },
            environment: EnvironmentBinding {
                kind: "local".to_string(),
                id: EvidenceId::parse("dev").unwrap(),
                digest: Sha256Digest::parse(
                    "sha256:5555555555555555555555555555555555555555555555555555555555555555",
                )
                .unwrap(),
            },
            vantage_point: VantagePoint {
                kind: "localhost".to_string(),
                identity: "127.0.0.1".to_string(),
            },
            capability: CapabilityBinding {
                manifest_id: EvidenceId::parse("vcap-test").unwrap(),
                manifest_digest: Sha256Digest::parse(
                    "sha256:6666666666666666666666666666666666666666666666666666666666666666",
                )
                .unwrap(),
                instance_id: EvidenceId::parse("vcinst-test").unwrap(),
                instance_digest: Sha256Digest::parse(
                    "sha256:7777777777777777777777777777777777777777777777777777777777777777",
                )
                .unwrap(),
            },
            provenance: TrustedProvenance::planr_observed_execution("exec-test").unwrap(),
            observations: vec![ObservationResult {
                requirement_id: EvidenceId::parse("obs-test").unwrap(),
                observation_type: NamespacedIdentifier::parse("planr.test").unwrap(),
                outcome: AttemptStatus::Passed,
                predicate: Map::new(),
                actual: Map::new(),
            }],
            attempt_ids: vec![EvidenceId::parse("eatt-test").unwrap()],
            retry_history: vec![],
            artifacts: vec![],
            raw_result: RawResultRef {
                kind: "inline".to_string(),
                digest: Sha256Digest::parse(
                    "sha256:9999999999999999999999999999999999999999999999999999999999999999",
                )
                .unwrap(),
                artifact_id: None,
                extra: Map::new(),
            },
            config_digest: Sha256Digest::parse(
                "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
            fixture_disclosure: FixtureDisclosure {
                fixtures_used: false,
                mocks_used: false,
                fixture_refs: None,
                mock_refs: None,
            },
            permissions: PermissionState {
                network: "none".to_string(),
                filesystem: "none".to_string(),
                environment: None,
                secrets: None,
            },
            sandbox: SandboxState {
                mode: "test".to_string(),
                limits: SandboxLimits {
                    timeout_ms: 1,
                    stdout_bytes: 0,
                    stderr_bytes: 0,
                },
            },
            proof_gaps: vec![],
            started_at: "2026-07-28T12:00:00Z".to_string(),
            ended_at: "2026-07-28T12:00:01Z".to_string(),
        }
    }

    fn assert_validates_against_frozen_schema(value: &Value) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let validator = jsonschema::draft202012::options()
            .build(&schema)
            .expect("Evidence contract schema compiles");
        let errors = validator
            .iter_errors(value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "{errors:#?}");
    }

    fn assert_rejected_by_frozen_schema(value: &Value) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(
                "docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let validator = jsonschema::draft202012::options()
            .build(&schema)
            .expect("Evidence contract schema compiles");
        assert!(
            validator.iter_errors(value).next().is_some(),
            "fixture must be rejected by the frozen schema"
        );
    }
}
