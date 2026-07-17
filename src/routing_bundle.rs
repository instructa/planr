//! Provider-neutral contract between Planr core and routing-policy producers.
//!
//! Core owns this wire shape and its generic invariants. It deliberately does
//! not interpret policy ids, host names, model ids, effort values, or
//! capability names. The optional `planr-routing` package is the producer and
//! sole owner of those opinions.

use crate::agents::{AgentProfile, DefaultRoute, Route};
use anyhow::{Context, Result, anyhow, bail};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const ROUTING_BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const ROUTING_APPLICATION_RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingBundleV1 {
    pub schema_version: u32,
    pub bundle_id: String,
    pub policy_id: String,
    pub policy_version: String,
    pub generated_at: String,
    pub source: RoutingBundleSourceV1,
    #[serde(default)]
    pub requirements: Vec<RoutingHostRequirementV1>,
    #[serde(default)]
    pub profiles: BTreeMap<String, AgentProfile>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub route_default: Option<DefaultRoute>,
    #[serde(default)]
    pub artifacts: Vec<RoutingArtifactV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<RoutingEvaluationEvidenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<RoutingBundleSignatureV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingBundleSourceV1 {
    pub package: String,
    pub package_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RoutingRegistryProvenanceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingRegistryProvenanceV1 {
    pub entry: String,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingHostRequirementV1 {
    pub host: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingArtifactV1 {
    pub path: String,
    pub media_type: String,
    pub mode: RoutingArtifactModeV1,
    #[serde(flatten)]
    pub payload: RoutingArtifactPayloadV1,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingArtifactModeV1 {
    Create,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RoutingArtifactPayloadV1 {
    Inline(RoutingInlineContentV1),
    Reference(RoutingContentReferenceV1),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingInlineContentV1 {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingContentReferenceV1 {
    pub content_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingEvaluationEvidenceV1 {
    #[serde(default)]
    pub evaluation_ids: Vec<String>,
    pub status: RoutingEvidenceStatusV1,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingEvidenceStatusV1 {
    Unverified,
    Experimental,
    Verified,
    Recommended,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingBundleSignatureV1 {
    pub algorithm: RoutingSignatureAlgorithmV1,
    pub signer: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingBundleTrustAnchorV1 {
    pub signer: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RoutingSignatureAlgorithmV1 {
    Ed25519,
}

/// Durable evidence written by core after preview or apply. Repository
/// identity is opaque so the contract does not require an absolute path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingApplicationRecordV1 {
    pub schema_version: u32,
    pub bundle_id: String,
    pub bundle_sha256: String,
    pub repository_id: String,
    pub previewed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<RoutingArtifactReceiptV1>,
    #[serde(default)]
    pub conflicts: Vec<RoutingArtifactConflictV1>,
    #[serde(default)]
    pub declared_routes: Vec<RoutingDeclaredRouteEvidenceV1>,
    #[serde(default)]
    pub effective_routes: Vec<RoutingEffectiveRouteEvidenceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingArtifactReceiptV1 {
    pub path: String,
    pub proposed_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_sha256: Option<String>,
    pub outcome: RoutingArtifactOutcomeV1,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingArtifactOutcomeV1 {
    Planned,
    Created,
    Replaced,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingArtifactConflictV1 {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingDeclaredRouteEvidenceV1 {
    pub selector: String,
    pub profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingEffectiveRouteEvidenceV1 {
    pub item_id: String,
    pub profile: String,
    pub observed_client: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingBundleError {
    UnsupportedSchemaVersion { found: u32 },
    InvalidField { field: String, reason: String },
    UnknownProfile { field: String, profile: String },
    DuplicateArtifactPath { path: String },
    ArtifactPathCollision { parent: String, child: String },
    ArtifactDigestMismatch { path: String },
    InvalidSignature,
    SignatureTrustRequired,
    SignatureSignerMismatch,
}

impl fmt::Display for RoutingBundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { found } => write!(
                formatter,
                "unsupported routing bundle schema version `{found}`; expected `{ROUTING_BUNDLE_SCHEMA_VERSION}`"
            ),
            Self::InvalidField { field, reason } => {
                write!(
                    formatter,
                    "invalid routing bundle field `{field}`: {reason}"
                )
            }
            Self::UnknownProfile { field, profile } => write!(
                formatter,
                "routing bundle field `{field}` references unknown profile `{profile}`"
            ),
            Self::DuplicateArtifactPath { path } => {
                write!(
                    formatter,
                    "routing bundle contains duplicate artifact path `{path}`"
                )
            }
            Self::ArtifactPathCollision { parent, child } => write!(
                formatter,
                "routing bundle artifact path `{parent}` is a parent of `{child}`"
            ),
            Self::ArtifactDigestMismatch { path } => write!(
                formatter,
                "routing bundle artifact `{path}` content does not match its sha256"
            ),
            Self::InvalidSignature => write!(formatter, "routing bundle signature is invalid"),
            Self::SignatureTrustRequired => write!(
                formatter,
                "signed routing bundle requires an external trusted signer and public key"
            ),
            Self::SignatureSignerMismatch => write!(
                formatter,
                "routing bundle signer does not match the external trust anchor"
            ),
        }
    }
}

impl Error for RoutingBundleError {}

impl RoutingBundleV1 {
    pub fn parse_json(raw: &str) -> Result<Self, RoutingBundleError> {
        serde_json::from_str(raw).map_err(|error| RoutingBundleError::InvalidField {
            field: "bundle".to_string(),
            reason: error.to_string(),
        })
    }

    fn validate_structure(&self) -> Result<(), RoutingBundleError> {
        if self.schema_version != ROUTING_BUNDLE_SCHEMA_VERSION {
            return Err(RoutingBundleError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        required("bundle_id", &self.bundle_id)?;
        required("policy_id", &self.policy_id)?;
        required("policy_version", &self.policy_version)?;
        required("generated_at", &self.generated_at)?;
        required("source.package", &self.source.package)?;
        required("source.package_version", &self.source.package_version)?;

        if let Some(registry) = &self.source.registry {
            required("source.registry.entry", &registry.entry)?;
            validate_sha256("source.registry.manifest_sha256", &registry.manifest_sha256)?;
        }

        let mut hosts = BTreeSet::new();
        for (requirement_index, requirement) in self.requirements.iter().enumerate() {
            let host_field = format!("requirements[{requirement_index}].host");
            required(&host_field, &requirement.host)?;
            if !hosts.insert(requirement.host.as_str()) {
                return invalid(&host_field, "host requirement is duplicated");
            }
            let mut capabilities = BTreeSet::new();
            for (capability_index, capability) in requirement.capabilities.iter().enumerate() {
                let field =
                    format!("requirements[{requirement_index}].capabilities[{capability_index}]");
                required(&field, capability)?;
                if !capabilities.insert(capability.as_str()) {
                    return invalid(&field, "capability is duplicated for this host");
                }
            }
        }

        for (profile_id, profile) in &self.profiles {
            required("profiles.<id>", profile_id)?;
            required(&format!("profiles.{profile_id}.client"), &profile.client)?;
            required(&format!("profiles.{profile_id}.model"), &profile.model)?;
        }
        for (route_index, route) in self.routes.iter().enumerate() {
            validate_route_profile(
                &self.profiles,
                &format!("routes[{route_index}].profile"),
                &route.profile,
            )?;
            for (fallback_index, fallback) in route.fallbacks.iter().enumerate() {
                validate_route_profile(
                    &self.profiles,
                    &format!("routes[{route_index}].fallbacks[{fallback_index}]"),
                    fallback,
                )?;
            }
        }
        if let Some(default) = &self.route_default {
            validate_route_profile(&self.profiles, "route_default.profile", &default.profile)?;
            for (fallback_index, fallback) in default.fallbacks.iter().enumerate() {
                validate_route_profile(
                    &self.profiles,
                    &format!("route_default.fallbacks[{fallback_index}]"),
                    fallback,
                )?;
            }
        }

        let mut artifact_paths = BTreeSet::new();
        for (artifact_index, artifact) in self.artifacts.iter().enumerate() {
            required(&format!("artifacts[{artifact_index}].path"), &artifact.path)?;
            required(
                &format!("artifacts[{artifact_index}].media_type"),
                &artifact.media_type,
            )?;
            validate_sha256(
                &format!("artifacts[{artifact_index}].sha256"),
                &artifact.sha256,
            )?;
            if !artifact_paths.insert(artifact.path.as_str()) {
                return Err(RoutingBundleError::DuplicateArtifactPath {
                    path: artifact.path.clone(),
                });
            }
            match &artifact.payload {
                RoutingArtifactPayloadV1::Inline(inline) => {
                    if sha256(inline.content.as_bytes()) != artifact.sha256 {
                        return Err(RoutingBundleError::ArtifactDigestMismatch {
                            path: artifact.path.clone(),
                        });
                    }
                }
                RoutingArtifactPayloadV1::Reference(reference) => required(
                    &format!("artifacts[{artifact_index}].content_ref"),
                    &reference.content_ref,
                )?,
            }
        }
        for child in &artifact_paths {
            let mut ancestor = Path::new(child).parent();
            while let Some(parent) = ancestor {
                if let Some(parent) = parent.to_str()
                    && artifact_paths.contains(parent)
                {
                    return Err(RoutingBundleError::ArtifactPathCollision {
                        parent: parent.to_string(),
                        child: (*child).to_string(),
                    });
                }
                ancestor = parent.parent();
            }
        }

        if let Some(evidence) = &self.evidence {
            let mut ids = BTreeSet::new();
            for (index, evaluation_id) in evidence.evaluation_ids.iter().enumerate() {
                let field = format!("evidence.evaluation_ids[{index}]");
                required(&field, evaluation_id)?;
                if !ids.insert(evaluation_id.as_str()) {
                    return invalid(&field, "evaluation id is duplicated");
                }
            }
            if matches!(
                evidence.status,
                RoutingEvidenceStatusV1::Verified | RoutingEvidenceStatusV1::Recommended
            ) {
                if evidence.evaluation_ids.is_empty() {
                    return invalid(
                        "evidence.evaluation_ids",
                        "verified or recommended evidence requires at least one evaluation id",
                    );
                }
                if self.signature.is_none() {
                    return invalid(
                        "evidence.status",
                        "verified or recommended evidence requires an externally trusted bundle signature",
                    );
                }
            }
        }
        if let Some(signature) = &self.signature {
            required("signature.signer", &signature.signer)?;
            validate_hex("signature.value", &signature.value, 128)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), RoutingBundleError> {
        self.validate_with_trust(None)
    }

    pub fn validate_with_trust(
        &self,
        trust: Option<&RoutingBundleTrustAnchorV1>,
    ) -> Result<(), RoutingBundleError> {
        self.validate_structure()?;
        self.verify_signature(trust)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RoutingBundleError> {
        serde_json::to_vec(self).map_err(|error| RoutingBundleError::InvalidField {
            field: "bundle".to_string(),
            reason: error.to_string(),
        })
    }

    pub fn digest(&self) -> Result<String, RoutingBundleError> {
        self.canonical_bytes().map(|bytes| sha256(&bytes))
    }

    fn signing_bytes(&self) -> Result<Vec<u8>, RoutingBundleError> {
        let mut unsigned = self.clone();
        unsigned.signature = None;
        unsigned.canonical_bytes()
    }

    fn verify_signature(
        &self,
        trust: Option<&RoutingBundleTrustAnchorV1>,
    ) -> Result<(), RoutingBundleError> {
        let Some(signature) = &self.signature else {
            return Ok(());
        };
        let trust = trust.ok_or(RoutingBundleError::SignatureTrustRequired)?;
        if signature.signer != trust.signer {
            return Err(RoutingBundleError::SignatureSignerMismatch);
        }
        let key_bytes =
            decode_hex::<32>(&trust.public_key).ok_or(RoutingBundleError::InvalidSignature)?;
        let signature_bytes =
            decode_hex::<64>(&signature.value).ok_or(RoutingBundleError::InvalidSignature)?;
        let key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|_| RoutingBundleError::InvalidSignature)?;
        key.verify(
            &self.signing_bytes()?,
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_| RoutingBundleError::InvalidSignature)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingArtifactActionV1 {
    Create,
    Replace,
    Unchanged,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingArtifactPreviewV1 {
    pub path: String,
    pub proposed_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_sha256: Option<String>,
    pub action: RoutingArtifactActionV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingBundlePreviewV1 {
    pub bundle_id: String,
    pub bundle_sha256: String,
    pub artifacts: Vec<RoutingArtifactPreviewV1>,
    pub conflicts: Vec<RoutingArtifactConflictV1>,
}

impl RoutingBundlePreviewV1 {
    pub fn is_applicable(&self) -> bool {
        self.conflicts.is_empty()
    }
}

impl RoutingApplicationRecordV1 {
    pub fn validate(&self) -> Result<(), RoutingBundleError> {
        if self.schema_version != ROUTING_APPLICATION_RECORD_SCHEMA_VERSION {
            return invalid(
                "application_record.schema_version",
                &format!(
                    "unsupported version `{}`; expected `{ROUTING_APPLICATION_RECORD_SCHEMA_VERSION}`",
                    self.schema_version
                ),
            );
        }
        required("application_record.bundle_id", &self.bundle_id)?;
        validate_sha256("application_record.bundle_sha256", &self.bundle_sha256)?;
        required("application_record.repository_id", &self.repository_id)?;
        required("application_record.previewed_at", &self.previewed_at)?;
        if let Some(applied_at) = &self.applied_at {
            required("application_record.applied_at", applied_at)?;
        }
        let mut artifact_paths = BTreeSet::new();
        for (index, artifact) in self.artifacts.iter().enumerate() {
            required(
                &format!("application_record.artifacts[{index}].path"),
                &artifact.path,
            )?;
            if !artifact_paths.insert(artifact.path.as_str()) {
                return invalid(
                    &format!("application_record.artifacts[{index}].path"),
                    "artifact receipt path is duplicated",
                );
            }
            validate_sha256(
                &format!("application_record.artifacts[{index}].proposed_sha256"),
                &artifact.proposed_sha256,
            )?;
            if let Some(previous) = &artifact.previous_sha256 {
                validate_sha256(
                    &format!("application_record.artifacts[{index}].previous_sha256"),
                    previous,
                )?;
            }
        }
        for (index, conflict) in self.conflicts.iter().enumerate() {
            required(
                &format!("application_record.conflicts[{index}].path"),
                &conflict.path,
            )?;
            required(
                &format!("application_record.conflicts[{index}].reason"),
                &conflict.reason,
            )?;
        }
        for (index, route) in self.declared_routes.iter().enumerate() {
            required(
                &format!("application_record.declared_routes[{index}].selector"),
                &route.selector,
            )?;
            required(
                &format!("application_record.declared_routes[{index}].profile"),
                &route.profile,
            )?;
        }
        for (index, route) in self.effective_routes.iter().enumerate() {
            required(
                &format!("application_record.effective_routes[{index}].item_id"),
                &route.item_id,
            )?;
            required(
                &format!("application_record.effective_routes[{index}].profile"),
                &route.profile,
            )?;
            required(
                &format!("application_record.effective_routes[{index}].observed_client"),
                &route.observed_client,
            )?;
            if let Some(model) = &route.observed_model {
                required(
                    &format!("application_record.effective_routes[{index}].observed_model"),
                    model,
                )?;
            }
            if let Some(effort) = &route.observed_effort {
                required(
                    &format!("application_record.effective_routes[{index}].observed_effort"),
                    effort,
                )?;
            }
        }
        Ok(())
    }
}

fn validate_route_profile(
    profiles: &BTreeMap<String, AgentProfile>,
    field: &str,
    profile: &str,
) -> Result<(), RoutingBundleError> {
    required(field, profile)?;
    if profiles.contains_key(profile) {
        Ok(())
    } else {
        Err(RoutingBundleError::UnknownProfile {
            field: field.to_string(),
            profile: profile.to_string(),
        })
    }
}

fn required(field: &str, value: &str) -> Result<(), RoutingBundleError> {
    if value.trim().is_empty() {
        invalid(field, "must not be blank")
    } else {
        Ok(())
    }
}

fn validate_sha256(field: &str, value: &str) -> Result<(), RoutingBundleError> {
    validate_hex(field, value, 64)
}

fn validate_hex(field: &str, value: &str, length: usize) -> Result<(), RoutingBundleError> {
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid(field, &format!("must be {length} hexadecimal characters"));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return invalid(field, "must use canonical lowercase hexadecimal");
    }
    Ok(())
}

fn invalid<T>(field: &str, reason: &str) -> Result<T, RoutingBundleError> {
    Err(RoutingBundleError::InvalidField {
        field: field.to_string(),
        reason: reason.to_string(),
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut decoded = [0_u8; N];
    for (index, output) in decoded.iter_mut().enumerate() {
        *output = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(decoded)
}

pub fn load_bundle(
    path: &Path,
    trust: Option<&RoutingBundleTrustAnchorV1>,
) -> Result<RoutingBundleV1> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("cannot read routing bundle {}", path.display()))?;
    let bundle = RoutingBundleV1::parse_json(&raw).map_err(anyhow::Error::new)?;
    match trust {
        Some(trust) => bundle.validate_with_trust(Some(trust)),
        None => bundle.validate(),
    }
    .map_err(anyhow::Error::new)?;
    Ok(bundle)
}

pub fn preview_bundle(
    root: &Path,
    bundle: &RoutingBundleV1,
    trust: Option<&RoutingBundleTrustAnchorV1>,
) -> Result<RoutingBundlePreviewV1> {
    match trust {
        Some(trust) => bundle.validate_with_trust(Some(trust)),
        None => bundle.validate(),
    }
    .map_err(anyhow::Error::new)?;
    let mut artifacts = Vec::with_capacity(bundle.artifacts.len());
    let mut conflicts = Vec::new();
    for artifact in &bundle.artifacts {
        let target = validated_repository_target(root, &artifact.path)?;
        let proposed = inline_content(artifact)?;
        let current = match fs::read(&target) {
            Ok(bytes) => Some(sha256(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("cannot inspect {}", target.display()));
            }
        };
        let action = match current.as_deref() {
            Some(current) if current == artifact.sha256 => RoutingArtifactActionV1::Unchanged,
            Some(_) if artifact.mode == RoutingArtifactModeV1::Replace => {
                RoutingArtifactActionV1::Replace
            }
            Some(_) => RoutingArtifactActionV1::Conflict,
            None => RoutingArtifactActionV1::Create,
        };
        if action == RoutingArtifactActionV1::Conflict {
            conflicts.push(RoutingArtifactConflictV1 {
                path: artifact.path.clone(),
                reason: "existing content differs and artifact mode is create".to_string(),
            });
        }
        debug_assert_eq!(sha256(proposed), artifact.sha256);
        artifacts.push(RoutingArtifactPreviewV1 {
            path: artifact.path.clone(),
            proposed_sha256: artifact.sha256.clone(),
            current_sha256: current,
            action,
        });
    }
    Ok(RoutingBundlePreviewV1 {
        bundle_id: bundle.bundle_id.clone(),
        bundle_sha256: bundle.digest().map_err(anyhow::Error::new)?,
        artifacts,
        conflicts,
    })
}

pub fn apply_bundle(
    root: &Path,
    bundle: &RoutingBundleV1,
    trust: Option<&RoutingBundleTrustAnchorV1>,
) -> Result<(RoutingBundlePreviewV1, Vec<RoutingArtifactReceiptV1>)> {
    let preview = preview_bundle(root, bundle, trust)?;
    if !preview.is_applicable() {
        bail!(
            "routing bundle has {} conflict(s); apply refused before writes",
            preview.conflicts.len()
        );
    }

    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("cannot canonicalize repository root {}", root.display()))?;
    let mut staged: Vec<StagedArtifact> = Vec::new();
    let mut created_directories: Vec<PathBuf> = Vec::new();
    for (artifact, artifact_preview) in bundle.artifacts.iter().zip(&preview.artifacts) {
        if artifact_preview.action == RoutingArtifactActionV1::Unchanged {
            continue;
        }
        let stage_result = (|| -> Result<StagedArtifact> {
            let target = validated_repository_target(root, &artifact.path)?;
            let parent = target.parent().ok_or_else(|| {
                anyhow!(
                    "routing artifact target has no parent: {}",
                    target.display()
                )
            })?;
            create_missing_directories(&canonical_root, parent, &mut created_directories)?;
            reject_symlink_components(root, Path::new(&artifact.path))?;
            let temporary = parent.join(format!(".planr-routing-{}.tmp", uuid::Uuid::new_v4()));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            if let Err(error) = file
                .write_all(inline_content(artifact)?)
                .and_then(|_| file.sync_all())
            {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
            Ok(StagedArtifact {
                target,
                temporary,
                action: artifact_preview.action,
                expected_current_sha256: artifact_preview.current_sha256.clone(),
            })
        })();
        match stage_result {
            Ok(stage) => staged.push(stage),
            Err(error) => {
                if let Err(rollback) = rollback_transaction(&staged, &[], &created_directories) {
                    return Err(error).context(format!("routing rollback also failed: {rollback}"));
                }
                return Err(error);
            }
        }
    }

    let mut committed: Vec<CommittedArtifact> = Vec::new();
    for stage in &staged {
        let result = commit_staged(stage);
        match result {
            Ok(commit) => committed.push(commit),
            Err(error) => {
                if let Err(rollback) =
                    rollback_transaction(&staged, &committed, &created_directories)
                {
                    return Err(error).context(format!("routing rollback also failed: {rollback}"));
                }
                return Err(error);
            }
        }
    }
    if let Err(error) = cleanup_backups(&committed) {
        if let Err(rollback) = rollback_transaction(&staged, &committed, &created_directories) {
            return Err(error).context(format!("routing rollback also failed: {rollback}"));
        }
        return Err(error);
    }

    let receipts = preview
        .artifacts
        .iter()
        .map(|artifact| RoutingArtifactReceiptV1 {
            path: artifact.path.clone(),
            proposed_sha256: artifact.proposed_sha256.clone(),
            previous_sha256: artifact.current_sha256.clone(),
            outcome: match artifact.action {
                RoutingArtifactActionV1::Create => RoutingArtifactOutcomeV1::Created,
                RoutingArtifactActionV1::Replace => RoutingArtifactOutcomeV1::Replaced,
                RoutingArtifactActionV1::Unchanged => RoutingArtifactOutcomeV1::Unchanged,
                RoutingArtifactActionV1::Conflict => unreachable!("conflicts stop before apply"),
            },
        })
        .collect();
    Ok((preview, receipts))
}

fn inline_content(artifact: &RoutingArtifactV1) -> Result<&[u8]> {
    match &artifact.payload {
        RoutingArtifactPayloadV1::Inline(inline) => Ok(inline.content.as_bytes()),
        RoutingArtifactPayloadV1::Reference(_) => bail!(
            "routing artifact `{}` uses unresolved content_ref; core apply requires self-contained content",
            artifact.path
        ),
    }
}

fn validated_repository_target(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("cannot canonicalize repository root {}", root.display()))?;
    let path = Path::new(relative);
    if path.is_absolute() || relative.starts_with('~') {
        bail!("routing artifact target `{relative}` must be repository-relative");
    }
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow!("routing artifact target `{relative}` is not UTF-8")),
            _ => Err(anyhow!(
                "routing artifact target `{relative}` contains traversal or non-normal components"
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    let normalized = components.join("/");
    if normalized != relative {
        bail!("routing artifact target `{relative}` is not normalized as `{normalized}`");
    }
    if !allowed_repository_target(&normalized) {
        bail!(
            "routing artifact target `{relative}` is outside the repository allowlist; user/global configuration and .codex/config.toml are forbidden"
        );
    }
    reject_symlink_components(&root, path)?;
    Ok(root.join(path))
}

fn allowed_repository_target(path: &str) -> bool {
    [
        ".planr/",
        ".codex/agents/",
        ".codex/skills/",
        ".claude/agents/",
        ".claude/skills/",
        ".cursor/agents/",
        ".cursor/skills/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix) && path.len() > prefix.len())
        && path != ".codex/config.toml"
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<()> {
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            bail!("routing artifact path contains a non-normal component");
        };
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "routing artifact target crosses symlink `{}`",
                    cursor.display()
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

struct StagedArtifact {
    target: PathBuf,
    temporary: PathBuf,
    action: RoutingArtifactActionV1,
    expected_current_sha256: Option<String>,
}

struct CommittedArtifact {
    target: PathBuf,
    backup: Option<PathBuf>,
    previous_content: Option<Vec<u8>>,
}

fn commit_staged(stage: &StagedArtifact) -> Result<CommittedArtifact> {
    match stage.action {
        RoutingArtifactActionV1::Create => {
            fs::hard_link(&stage.temporary, &stage.target).with_context(|| {
                format!(
                    "routing target appeared after preview; refusing overwrite: {}",
                    stage.target.display()
                )
            })?;
            if let Err(error) = fs::remove_file(&stage.temporary) {
                let _ = fs::remove_file(&stage.target);
                return Err(error.into());
            }
            Ok(CommittedArtifact {
                target: stage.target.clone(),
                backup: None,
                previous_content: None,
            })
        }
        RoutingArtifactActionV1::Replace => {
            let current = fs::read(&stage.target).with_context(|| {
                format!("cannot recheck {} before replace", stage.target.display())
            })?;
            let current_sha256 = sha256(&current);
            if stage.expected_current_sha256.as_deref() != Some(current_sha256.as_str()) {
                bail!(
                    "routing target changed after preview; refusing replace: {}",
                    stage.target.display()
                );
            }
            let backup = stage
                .target
                .with_extension(format!("planr-backup-{}", uuid::Uuid::new_v4()));
            fs::rename(&stage.target, &backup)?;
            if let Err(error) = fs::rename(&stage.temporary, &stage.target) {
                let _ = fs::rename(&backup, &stage.target);
                let _ = fs::remove_file(&stage.temporary);
                return Err(error.into());
            }
            Ok(CommittedArtifact {
                target: stage.target.clone(),
                backup: Some(backup),
                previous_content: Some(current),
            })
        }
        RoutingArtifactActionV1::Unchanged | RoutingArtifactActionV1::Conflict => {
            unreachable!("only mutable staged actions are committed")
        }
    }
}

fn create_missing_directories(
    root: &Path,
    parent: &Path,
    created: &mut Vec<PathBuf>,
) -> Result<()> {
    create_missing_directories_with(root, parent, created, |path| fs::create_dir(path))
}

fn create_missing_directories_with<F>(
    root: &Path,
    parent: &Path,
    created: &mut Vec<PathBuf>,
    mut create: F,
) -> Result<()>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let mut missing = Vec::new();
    let mut cursor = parent;
    while cursor != root {
        if cursor.exists() {
            break;
        }
        if !cursor.starts_with(root) {
            bail!("routing artifact parent escaped repository root");
        }
        missing.push(cursor.to_path_buf());
        cursor = cursor
            .parent()
            .ok_or_else(|| anyhow!("routing artifact parent has no repository ancestor"))?;
    }
    missing.reverse();
    for directory in missing {
        match create(&directory) {
            Ok(()) => created.push(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn cleanup_backups(committed: &[CommittedArtifact]) -> Result<()> {
    cleanup_backups_with(committed, |path| fs::remove_file(path))
}

fn cleanup_backups_with<F>(committed: &[CommittedArtifact], mut remove: F) -> Result<()>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let mut errors = Vec::new();
    for backup in committed.iter().filter_map(|commit| commit.backup.as_ref()) {
        if let Err(error) = remove(backup)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            errors.push(format!("remove backup {}: {error}", backup.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        bail!(errors.join("; "))
    }
}

fn rollback_transaction(
    staged: &[StagedArtifact],
    committed: &[CommittedArtifact],
    created_directories: &[PathBuf],
) -> Result<()> {
    let mut errors = Vec::new();
    for staged_artifact in staged {
        if let Err(error) = fs::remove_file(&staged_artifact.temporary)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            errors.push(format!(
                "remove temporary {}: {error}",
                staged_artifact.temporary.display()
            ));
        }
    }
    for commit in committed.iter().rev() {
        if let Err(error) = fs::remove_file(&commit.target)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            errors.push(format!(
                "remove target {}: {error}",
                commit.target.display()
            ));
        }
        if let Some(backup) = &commit.backup {
            match fs::rename(backup, &commit.target) {
                Ok(()) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => errors.push(format!(
                    "restore backup {} to {}: {error}",
                    backup.display(),
                    commit.target.display()
                )),
            }
        }
        if let Some(previous) = &commit.previous_content
            && let Err(error) = fs::write(&commit.target, previous)
        {
            errors.push(format!(
                "restore previous content {}: {error}",
                commit.target.display()
            ));
        }
    }
    for directory in created_directories.iter().rev() {
        if let Err(error) = fs::remove_dir(directory)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            errors.push(format!("remove directory {}: {error}", directory.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        bail!(errors.join("; "))
    }
}

#[cfg(test)]
mod tests;
