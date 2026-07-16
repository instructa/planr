//! Declarative preset registry verification and immutable offline caching.
//!
//! Registry content is never executable and never becomes trusted merely
//! because a manifest carries its own key. Ed25519 signatures are checked
//! against a separately provisioned repository-local trust store.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const TRUST_STORE_PATH: &str = ".planr/registry/trusted-maintainers.toml";
pub const CACHE_ROOT: &str = ".planr/registry/cache";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub generated_at_unix: u64,
    #[serde(default)]
    pub entries: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub id: String,
    pub version: String,
    pub kind: RegistryEntryKind,
    pub lifecycle: RegistryLifecycle,
    pub verification_status: VerificationStatus,
    pub verified_at_unix: u64,
    pub review_at_unix: u64,
    #[serde(default)]
    pub compatible_hosts: Vec<String>,
    pub min_planr_version: Option<String>,
    pub max_planr_version: Option<String>,
    pub verification_path: String,
    pub evaluation: Option<RegistryEvaluationRef>,
    pub replacement: Option<String>,
    pub revocation_reason: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<RegistryArtifact>,
    pub signature: Option<RegistrySignature>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEvaluationRef {
    pub policy_id: String,
    pub policy_version: String,
    pub binding_id: String,
    pub binding_version: String,
    pub suite_id: String,
    pub suite_version: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryEntryKind {
    Policy,
    HostBinding,
    Pack,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryLifecycle {
    Published,
    Deprecated,
    Revoked,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationStatus {
    Experimental,
    Verified,
    Recommended,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryArtifact {
    pub path: String,
    pub kind: RegistryArtifactKind,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryArtifactKind {
    Policy,
    HostBinding,
    Documentation,
    Verification,
    Manifest,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrySignature {
    pub signer: String,
    pub algorithm: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintainerTrustStore {
    pub schema_version: u32,
    #[serde(default)]
    pub maintainers: Vec<TrustedMaintainer>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedMaintainer {
    pub id: String,
    pub public_key: String,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    Current,
    Stale,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifiedRegistryEntry {
    pub registry_id: String,
    pub registry_version: String,
    pub manifest_sha256: String,
    pub entry: RegistryEntry,
    pub integrity_verified: bool,
    pub signature_verified: bool,
    pub trusted_maintainer: bool,
    pub compatible: bool,
    pub freshness: FreshnessState,
    pub effective_status: String,
    pub recommended: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryImportPreview {
    pub action: String,
    pub cache_path: String,
    pub entry: VerifiedRegistryEntry,
    pub artifacts: Vec<RegistryImportArtifact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryImportArtifact {
    pub source: String,
    pub target: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct RegistryVerificationOptions<'a> {
    pub trust_store: Option<&'a MaintainerTrustStore>,
    pub now_unix: u64,
    pub planr_version: &'a str,
    pub host: Option<&'a str>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CacheReceipt {
    schema_version: u32,
    cached_at_unix: u64,
    registry_id: String,
    registry_version: String,
    entry_id: String,
    entry_version: String,
    manifest_sha256: String,
    verified_at_unix: u64,
    review_at_unix: u64,
    lifecycle: RegistryLifecycle,
    host: Option<String>,
    artifacts: Vec<RegistryArtifact>,
}

#[derive(Debug, Serialize)]
struct SignableEntry<'a> {
    registry_id: &'a str,
    registry_version: &'a str,
    schema_version: u32,
    entry: UnsignedEntry<'a>,
}

#[derive(Debug, Serialize)]
struct UnsignedEntry<'a> {
    id: &'a str,
    version: &'a str,
    kind: RegistryEntryKind,
    lifecycle: RegistryLifecycle,
    verification_status: VerificationStatus,
    verified_at_unix: u64,
    review_at_unix: u64,
    compatible_hosts: &'a [String],
    min_planr_version: &'a Option<String>,
    max_planr_version: &'a Option<String>,
    verification_path: &'a str,
    evaluation: &'a Option<RegistryEvaluationRef>,
    replacement: &'a Option<String>,
    revocation_reason: &'a Option<String>,
    artifacts: &'a [RegistryArtifact],
}

pub fn parse_manifest(raw: &str) -> Result<RegistryManifest, String> {
    let manifest: RegistryManifest =
        toml::from_str(raw).map_err(|error| format!("registry manifest parse failed: {error}"))?;
    if manifest.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported registry schema {}; expected {}",
            manifest.schema_version, REGISTRY_SCHEMA_VERSION
        ));
    }
    validate_identifier("registry id", &manifest.id)?;
    validate_metadata_value("registry version", &manifest.version)?;
    let mut ids = BTreeSet::new();
    for entry in &manifest.entries {
        validate_entry(entry)?;
        if !ids.insert(entry.id.as_str()) {
            return Err(format!(
                "duplicate registry entry id `{}`; one manifest version must resolve each id unambiguously",
                entry.id
            ));
        }
    }
    Ok(manifest)
}

pub fn parse_trust_store(raw: &str) -> Result<MaintainerTrustStore, String> {
    let store: MaintainerTrustStore = toml::from_str(raw)
        .map_err(|error| format!("maintainer trust store parse failed: {error}"))?;
    if store.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported maintainer trust schema {}; expected {}",
            store.schema_version, REGISTRY_SCHEMA_VERSION
        ));
    }
    let mut ids = BTreeSet::new();
    for maintainer in &store.maintainers {
        validate_identifier("maintainer id", &maintainer.id)?;
        if !ids.insert(maintainer.id.as_str()) {
            return Err(format!("duplicate trusted maintainer `{}`", maintainer.id));
        }
        decode_hex_exact(&maintainer.public_key, 32)
            .map_err(|error| format!("maintainer `{}` public key: {error}", maintainer.id))?;
    }
    Ok(store)
}

pub fn verify_entry(
    manifest_raw: &str,
    entry_id: &str,
    content_root: &Path,
    trust_store: Option<&MaintainerTrustStore>,
    now_unix: u64,
    planr_version: &str,
    host: Option<&str>,
) -> Result<VerifiedRegistryEntry, String> {
    let manifest = parse_manifest(manifest_raw)?;
    let entry = manifest
        .entries
        .iter()
        .find(|entry| entry.id == entry_id)
        .ok_or_else(|| format!("registry entry `{entry_id}` was not found"))?;
    if entry.lifecycle == RegistryLifecycle::Revoked {
        return Err(format!(
            "registry entry `{entry_id}` is revoked{}",
            entry
                .revocation_reason
                .as_deref()
                .map(|reason| format!(": {reason}"))
                .unwrap_or_default()
        ));
    }
    if entry.verified_at_unix > now_unix {
        return Err(format!(
            "registry entry `{entry_id}` has a verification timestamp in the future"
        ));
    }

    verify_artifacts(entry, content_root)?;
    let compatible = compatibility(entry, planr_version, host)?;
    if !compatible {
        return Err(format!(
            "registry entry `{entry_id}` is incompatible with Planr {planr_version}{}",
            host.map(|host| format!(" and host `{host}`"))
                .unwrap_or_default()
        ));
    }
    let evaluation_recommended = validate_entry_evaluation(entry, content_root, now_unix, host)?;

    let (signature_verified, trusted_maintainer, signature_reason) =
        verify_signature(&manifest, entry, trust_store)?;
    let freshness = if now_unix <= entry.review_at_unix {
        FreshnessState::Current
    } else {
        FreshnessState::Stale
    };
    let mut reasons = vec![signature_reason];
    let effective_status = if freshness == FreshnessState::Stale {
        reasons.push(format!(
            "verification review expired at Unix timestamp {}",
            entry.review_at_unix
        ));
        "stale"
    } else if entry.lifecycle == RegistryLifecycle::Deprecated {
        reasons.push(
            entry
                .replacement
                .as_deref()
                .map(|replacement| format!("deprecated; replacement is `{replacement}`"))
                .unwrap_or_else(|| "deprecated without a replacement".into()),
        );
        "deprecated"
    } else {
        match entry.verification_status {
            VerificationStatus::Experimental => "experimental",
            VerificationStatus::Verified => "verified",
            VerificationStatus::Recommended => {
                if signature_verified && trusted_maintainer && evaluation_recommended {
                    "recommended"
                } else {
                    if !signature_verified || !trusted_maintainer {
                        reasons.push(
                            "recommendation demoted because no trusted maintainer signature was verified"
                                .into(),
                        );
                    }
                    if !evaluation_recommended {
                        reasons.push(
                            "recommendation demoted because the current canonical evaluation report did not earn recommended"
                                .into(),
                        );
                    }
                    "verified"
                }
            }
        }
    }
    .to_string();

    Ok(VerifiedRegistryEntry {
        registry_id: manifest.id,
        registry_version: manifest.version,
        manifest_sha256: sha256(manifest_raw.as_bytes()),
        entry: entry.clone(),
        integrity_verified: true,
        signature_verified,
        trusted_maintainer,
        compatible,
        freshness,
        recommended: effective_status == "recommended",
        effective_status,
        reasons,
    })
}

pub fn import_entry(
    repository_root: &Path,
    manifest_raw: &str,
    entry_id: &str,
    content_root: &Path,
    options: RegistryVerificationOptions<'_>,
    confirm: bool,
) -> Result<RegistryImportPreview, String> {
    let verified = verify_entry(
        manifest_raw,
        entry_id,
        content_root,
        options.trust_store,
        options.now_unix,
        options.planr_version,
        options.host,
    )?;
    let cache_relative = PathBuf::from(CACHE_ROOT)
        .join(&verified.registry_id)
        .join(&verified.entry.id)
        .join(&verified.entry.version)
        .join(&verified.manifest_sha256);
    let cache_root = repository_root.join(&cache_relative);
    let artifacts = verified
        .entry
        .artifacts
        .iter()
        .map(|artifact| RegistryImportArtifact {
            source: artifact.path.clone(),
            target: cache_relative
                .join("content")
                .join(&artifact.path)
                .to_string_lossy()
                .replace('\\', "/"),
            sha256: artifact.sha256.clone(),
            size_bytes: artifact.size_bytes,
        })
        .collect::<Vec<_>>();

    if confirm {
        write_cache(
            repository_root,
            &cache_root,
            manifest_raw,
            content_root,
            &verified,
            options,
            options.now_unix,
        )?;
    }

    Ok(RegistryImportPreview {
        action: if confirm { "imported" } else { "preview" }.into(),
        cache_path: cache_relative.to_string_lossy().replace('\\', "/"),
        entry: verified,
        artifacts,
    })
}

pub fn list_cache(repository_root: &Path, now_unix: u64) -> Result<serde_json::Value, String> {
    let root = repository_root.join(CACHE_ROOT);
    if !root.exists() {
        return Ok(serde_json::json!({"cache_root": CACHE_ROOT, "entries": []}));
    }
    reject_symlink_components(repository_root, Path::new(CACHE_ROOT), false)?;
    let trust_store = load_cached_trust_store(repository_root)?;
    let mut receipts = Vec::new();
    collect_receipts(&root, &mut receipts)?;
    receipts.sort_by(|a, b| {
        (
            &a.1.registry_id,
            &a.1.entry_id,
            &a.1.entry_version,
            &a.1.manifest_sha256,
        )
            .cmp(&(
                &b.1.registry_id,
                &b.1.entry_id,
                &b.1.entry_version,
                &b.1.manifest_sha256,
            ))
    });
    let values = receipts
        .into_iter()
        .map(|(receipt_root, receipt)| {
            let anchored = verify_cached_entry(
                &root,
                &receipt_root,
                &receipt,
                trust_store.as_ref(),
                now_unix,
            );
            let (
                integrity_verified,
                integrity_error,
                signature_verified,
                trusted_maintainer,
                effective_status,
                recommended,
                freshness,
                usable,
            ) = match anchored {
                Ok(verified) => (
                    true,
                    None,
                    verified.signature_verified,
                    verified.trusted_maintainer,
                    verified.effective_status,
                    verified.recommended,
                    match verified.freshness {
                        FreshnessState::Current => "current",
                        FreshnessState::Stale => "stale",
                    },
                    true,
                ),
                Err(error) => (
                    false,
                    Some(error),
                    false,
                    false,
                    "corrupt".to_string(),
                    false,
                    if now_unix <= receipt.review_at_unix {
                        "current"
                    } else {
                        "stale"
                    },
                    false,
                ),
            };
            serde_json::json!({
                "registry_id": receipt.registry_id,
                "registry_version": receipt.registry_version,
                "entry_id": receipt.entry_id,
                "entry_version": receipt.entry_version,
                "manifest_sha256": receipt.manifest_sha256,
                "cached_at_unix": receipt.cached_at_unix,
                "verified_at_unix": receipt.verified_at_unix,
                "review_at_unix": receipt.review_at_unix,
                "freshness": freshness,
                "lifecycle": receipt.lifecycle,
                "integrity_verified_at_import": true,
                "integrity_verified": integrity_verified,
                "integrity_error": integrity_error,
                "usable": usable,
                "effective_status": effective_status,
                "recommended": recommended,
                "signature_verified": signature_verified,
                "trusted_maintainer": trusted_maintainer,
                "artifact_count": receipt.artifacts.len(),
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({"cache_root": CACHE_ROOT, "entries": values}))
}

pub fn signature_message(manifest: &RegistryManifest, entry: &RegistryEntry) -> Vec<u8> {
    let value = SignableEntry {
        registry_id: &manifest.id,
        registry_version: &manifest.version,
        schema_version: manifest.schema_version,
        entry: UnsignedEntry {
            id: &entry.id,
            version: &entry.version,
            kind: entry.kind,
            lifecycle: entry.lifecycle,
            verification_status: entry.verification_status,
            verified_at_unix: entry.verified_at_unix,
            review_at_unix: entry.review_at_unix,
            compatible_hosts: &entry.compatible_hosts,
            min_planr_version: &entry.min_planr_version,
            max_planr_version: &entry.max_planr_version,
            verification_path: &entry.verification_path,
            evaluation: &entry.evaluation,
            replacement: &entry.replacement,
            revocation_reason: &entry.revocation_reason,
            artifacts: &entry.artifacts,
        },
    };
    let canonical = serde_json::to_vec(&value).expect("registry signing payload serializes");
    let digest = sha256(&canonical);
    format!("planr-registry-entry-v1\n{digest}\n").into_bytes()
}

fn validate_entry(entry: &RegistryEntry) -> Result<(), String> {
    validate_identifier("entry id", &entry.id)?;
    validate_metadata_value("entry version", &entry.version)?;
    validate_metadata_value("verification path", &entry.verification_path)?;
    for host in &entry.compatible_hosts {
        validate_identifier("compatible host", host)?;
    }
    for (label, version) in [
        ("minimum Planr version", entry.min_planr_version.as_deref()),
        ("maximum Planr version", entry.max_planr_version.as_deref()),
    ] {
        if let Some(version) = version {
            validate_metadata_value(label, version)?;
        }
    }
    if let Some(evaluation) = entry.evaluation.as_ref() {
        for (label, identifier) in [
            ("evaluation policy id", evaluation.policy_id.as_str()),
            ("evaluation binding id", evaluation.binding_id.as_str()),
            ("evaluation suite id", evaluation.suite_id.as_str()),
        ] {
            validate_identifier(label, identifier)?;
        }
        for (label, version) in [
            (
                "evaluation policy version",
                evaluation.policy_version.as_str(),
            ),
            (
                "evaluation binding version",
                evaluation.binding_version.as_str(),
            ),
            (
                "evaluation suite version",
                evaluation.suite_version.as_str(),
            ),
        ] {
            validate_metadata_value(label, version)?;
        }
    }
    if let Some(replacement) = entry.replacement.as_deref() {
        validate_registry_reference("replacement", replacement)?;
    }
    if let Some(reason) = entry.revocation_reason.as_deref() {
        validate_metadata_value("revocation reason", reason)?;
    }
    if let Some(signature) = entry.signature.as_ref() {
        validate_identifier("signature signer", &signature.signer)?;
        if signature.algorithm != "ed25519" {
            return Err("registry signature algorithm must be `ed25519`".into());
        }
        decode_hex_exact(&signature.value, 64)
            .map_err(|error| format!("registry signature value: {error}"))?;
    }
    if entry.verified_at_unix > entry.review_at_unix {
        return Err(format!(
            "registry entry `{}` review date precedes its verification date",
            entry.id
        ));
    }
    if entry.lifecycle == RegistryLifecycle::Revoked && entry.revocation_reason.is_none() {
        return Err(format!(
            "revoked registry entry `{}` requires revocation_reason",
            entry.id
        ));
    }
    if entry.lifecycle == RegistryLifecycle::Deprecated && entry.replacement.is_none() {
        return Err(format!(
            "deprecated registry entry `{}` requires replacement",
            entry.id
        ));
    }
    if matches!(
        entry.verification_status,
        VerificationStatus::Verified | VerificationStatus::Recommended
    ) && entry.evaluation.is_none()
    {
        return Err(format!(
            "registry entry `{}` requires evaluation binding metadata",
            entry.id
        ));
    }
    let mut paths = BTreeSet::new();
    for artifact in &entry.artifacts {
        validate_metadata_value("artifact path", &artifact.path)?;
        normalized_relative_path(&artifact.path)?;
        if !paths.insert(artifact.path.as_str()) {
            return Err(format!(
                "registry entry `{}` declares duplicate artifact `{}`",
                entry.id, artifact.path
            ));
        }
        decode_hex_exact(&artifact.sha256, 32)
            .map_err(|error| format!("artifact `{}` checksum: {error}", artifact.path))?;
    }
    let verification = entry
        .artifacts
        .iter()
        .find(|artifact| artifact.path == entry.verification_path)
        .ok_or_else(|| {
            format!(
                "registry entry `{}` verification_path is not a declared artifact",
                entry.id
            )
        })?;
    if verification.kind != RegistryArtifactKind::Verification {
        return Err(format!(
            "registry entry `{}` verification_path must have kind `verification`",
            entry.id
        ));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    reject_secret_like(label, value)?;
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "{label} `{value}` must use only ASCII letters, digits, dash, underscore, or dot"
        ));
    }
    Ok(())
}

fn validate_metadata_value(label: &str, value: &str) -> Result<(), String> {
    reject_secret_like(label, value)?;
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err(format!("{label} contains invalid metadata characters"));
    }
    Ok(())
}

fn validate_registry_reference(label: &str, value: &str) -> Result<(), String> {
    validate_metadata_value(label, value)?;
    let mut parts = value.split('@');
    let id = parts.next().unwrap_or_default();
    validate_identifier(label, id)?;
    if let Some(version) = parts.next() {
        validate_metadata_value(&format!("{label} version"), version)?;
    }
    if parts.next().is_some() {
        return Err(format!("{label} must be an entry id or `entry-id@version`"));
    }
    Ok(())
}

fn reject_secret_like(label: &str, value: &str) -> Result<(), String> {
    if crate::secrets::looks_secret_like(value) {
        Err(format!("{label} contains secret-like content"))
    } else {
        Ok(())
    }
}

fn verify_artifacts(entry: &RegistryEntry, content_root: &Path) -> Result<(), String> {
    let canonical_root = content_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve registry content root: {error}"))?;
    for artifact in &entry.artifacts {
        let relative = normalized_relative_path(&artifact.path)?;
        reject_symlink_components(&canonical_root, &relative, true)?;
        let path = canonical_root.join(&relative);
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("failed to read artifact `{}`: {error}", artifact.path))?;
        if !metadata.is_file() {
            return Err(format!(
                "registry artifact `{}` is not a file",
                artifact.path
            ));
        }
        if metadata.len() != artifact.size_bytes {
            return Err(format!(
                "registry artifact `{}` size mismatch: expected {}, observed {}",
                artifact.path,
                artifact.size_bytes,
                metadata.len()
            ));
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read artifact `{}`: {error}", artifact.path))?;
        let observed = sha256(&bytes);
        if observed != artifact.sha256.to_ascii_lowercase() {
            return Err(format!(
                "registry artifact `{}` checksum mismatch: expected {}, observed {}",
                artifact.path, artifact.sha256, observed
            ));
        }
        verify_artifact_safety(artifact, &path, &bytes)?;
    }
    Ok(())
}

fn verify_artifact_safety(
    artifact: &RegistryArtifact,
    path: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|error| format!("failed to inspect artifact permissions: {error}"))?
            .permissions()
            .mode();
        if mode & 0o111 != 0 {
            return Err(format!(
                "registry artifact `{}` is executable; registry content must be declarative",
                artifact.path
            ));
        }
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        format!(
            "registry artifact `{}` is binary; registry content must be declarative text",
            artifact.path
        )
    })?;
    if crate::secrets::looks_secret_like(text) {
        return Err(format!(
            "registry artifact `{}` contains secret-like content",
            artifact.path
        ));
    }
    let extension = path.extension().and_then(|value| value.to_str());
    match artifact.kind {
        RegistryArtifactKind::Policy => {
            if extension != Some("toml") {
                return Err(format!(
                    "policy artifact `{}` must use the .toml extension",
                    artifact.path
                ));
            }
            let policy = crate::usage_policy::parse_policy(text)
                .map_err(|error| format!("policy artifact `{}`: {error}", artifact.path))?;
            let errors = crate::preset_catalog::validate_registry_policy(&policy);
            if !errors.is_empty() {
                return Err(format!(
                    "policy artifact `{}` is not registry-safe: {}",
                    artifact.path,
                    errors.join("; ")
                ));
            }
        }
        RegistryArtifactKind::HostBinding => {
            if extension != Some("toml") {
                return Err(format!(
                    "host-binding artifact `{}` must use the .toml extension",
                    artifact.path
                ));
            }
            let binding = crate::preset::parse_host_binding(text)
                .map_err(|error| format!("host-binding artifact `{}`: {error}", artifact.path))?;
            let errors = crate::preset_catalog::validate_registry_binding(&binding);
            if !errors.is_empty() {
                return Err(format!(
                    "host-binding artifact `{}` is not registry-safe: {}",
                    artifact.path,
                    errors.join("; ")
                ));
            }
        }
        RegistryArtifactKind::Documentation => {
            if extension != Some("md") {
                return Err(format!(
                    "documentation artifact `{}` must use the .md extension",
                    artifact.path
                ));
            }
        }
        RegistryArtifactKind::Verification => {
            if extension != Some("json") {
                return Err(format!(
                    "verification artifact `{}` must use the .json extension",
                    artifact.path
                ));
            }
            serde_json::from_str::<serde_json::Value>(text).map_err(|error| {
                format!(
                    "verification artifact `{}` is not valid JSON: {error}",
                    artifact.path
                )
            })?;
        }
        RegistryArtifactKind::Manifest => match extension {
            Some("toml") => {
                toml::from_str::<toml::Value>(text).map_err(|error| {
                    format!(
                        "manifest artifact `{}` is not valid TOML: {error}",
                        artifact.path
                    )
                })?;
            }
            Some("json") => {
                serde_json::from_str::<serde_json::Value>(text).map_err(|error| {
                    format!(
                        "manifest artifact `{}` is not valid JSON: {error}",
                        artifact.path
                    )
                })?;
            }
            _ => {
                return Err(format!(
                    "manifest artifact `{}` must use .toml or .json",
                    artifact.path
                ));
            }
        },
    }
    Ok(())
}

fn validate_entry_evaluation(
    entry: &RegistryEntry,
    content_root: &Path,
    now_unix: u64,
    host: Option<&str>,
) -> Result<bool, String> {
    let Some(evaluation) = entry.evaluation.as_ref() else {
        return Ok(false);
    };
    let policy_artifact = exactly_one_artifact(entry, RegistryArtifactKind::Policy)?;
    let binding_artifact = exactly_one_artifact(entry, RegistryArtifactKind::HostBinding)?;
    let verification_artifact = entry
        .artifacts
        .iter()
        .find(|artifact| artifact.path == entry.verification_path)
        .ok_or_else(|| "verification artifact disappeared after manifest validation".to_string())?;
    let policy_raw = fs::read_to_string(content_root.join(&policy_artifact.path))
        .map_err(|error| format!("failed to read evaluated policy artifact: {error}"))?;
    let binding_raw = fs::read_to_string(content_root.join(&binding_artifact.path))
        .map_err(|error| format!("failed to read evaluated binding artifact: {error}"))?;
    let verification_raw = fs::read_to_string(content_root.join(&verification_artifact.path))
        .map_err(|error| format!("failed to read evaluation report artifact: {error}"))?;
    let policy = crate::usage_policy::parse_policy(&policy_raw)
        .map_err(|error| format!("evaluated policy is invalid: {error}"))?;
    let binding = crate::preset::parse_host_binding(&binding_raw)
        .map_err(|error| format!("evaluated binding is invalid: {error}"))?;
    for (label, observed, expected) in [
        (
            "policy id",
            policy.id.as_str(),
            evaluation.policy_id.as_str(),
        ),
        (
            "policy version",
            policy.version.as_str(),
            evaluation.policy_version.as_str(),
        ),
        (
            "binding id",
            binding.id.as_str(),
            evaluation.binding_id.as_str(),
        ),
        (
            "binding version",
            binding.version.as_str(),
            evaluation.binding_version.as_str(),
        ),
    ] {
        if observed != expected {
            return Err(format!(
                "registry evaluation {label} mismatch: expected `{expected}`, observed `{observed}`"
            ));
        }
    }
    let pack = crate::preset_catalog::validate_pack(
        &policy,
        &binding,
        Some(&evaluation.policy_id),
        Some(&evaluation.binding_id),
    );
    if !pack.safe {
        return Err(format!(
            "registry evaluation references a non-safe pack: {}",
            pack.warnings.join("; ")
        ));
    }
    let validated = crate::preset_eval::validate_registry_evaluation(
        &verification_raw,
        &evaluation.policy_id,
        &policy_raw,
        &evaluation.binding_id,
        &binding_raw,
        now_unix,
        host,
    )?;
    let report: serde_json::Value = serde_json::from_str(&verification_raw)
        .map_err(|error| format!("preset evaluation report is not valid JSON: {error}"))?;
    let report = report.get("report").unwrap_or(&report);
    require_registry_report_string(report, "/suite/id", &evaluation.suite_id)?;
    require_registry_report_string(report, "/suite/version", &evaluation.suite_version)?;
    Ok(validated.recommended)
}

fn exactly_one_artifact(
    entry: &RegistryEntry,
    kind: RegistryArtifactKind,
) -> Result<&RegistryArtifact, String> {
    let mut matches = entry
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == kind);
    let first = matches.next().ok_or_else(|| {
        format!(
            "registry evaluation requires exactly one `{}` artifact",
            serde_json::to_value(kind)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "unknown".into())
        )
    })?;
    if matches.next().is_some() {
        return Err("registry evaluation contains duplicate policy or binding artifacts".into());
    }
    Ok(first)
}

fn require_registry_report_string(
    report: &serde_json::Value,
    pointer: &str,
    expected: &str,
) -> Result<(), String> {
    let observed = report.pointer(pointer).and_then(serde_json::Value::as_str);
    if observed == Some(expected) {
        Ok(())
    } else {
        Err(format!(
            "registry evaluation `{pointer}` mismatch: expected `{expected}`, observed `{}`",
            observed.unwrap_or("<missing>")
        ))
    }
}

fn compatibility(
    entry: &RegistryEntry,
    planr_version: &str,
    host: Option<&str>,
) -> Result<bool, String> {
    let current = parse_version(planr_version)?;
    if let Some(minimum) = entry.min_planr_version.as_deref() {
        if current < parse_version(minimum)? {
            return Ok(false);
        }
    }
    if let Some(maximum) = entry.max_planr_version.as_deref() {
        if current > parse_version(maximum)? {
            return Ok(false);
        }
    }
    if !entry.compatible_hosts.is_empty() {
        let host = host.ok_or_else(|| {
            format!(
                "registry entry `{}` requires an explicit host compatibility check",
                entry.id
            )
        })?;
        if !entry
            .compatible_hosts
            .iter()
            .any(|candidate| candidate == host)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_version(value: &str) -> Result<(u64, u64, u64), String> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let core = value.split(['-', '+']).next().unwrap_or(value);
    let mut parts = core.split('.');
    let major = parts
        .next()
        .ok_or_else(|| format!("invalid version `{value}`"))?
        .parse::<u64>()
        .map_err(|_| format!("invalid version `{value}`"))?;
    let minor = parts
        .next()
        .unwrap_or("0")
        .parse::<u64>()
        .map_err(|_| format!("invalid version `{value}`"))?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .parse::<u64>()
        .map_err(|_| format!("invalid version `{value}`"))?;
    if parts.next().is_some() {
        return Err(format!("invalid version `{value}`"));
    }
    Ok((major, minor, patch))
}

fn verify_signature(
    manifest: &RegistryManifest,
    entry: &RegistryEntry,
    trust_store: Option<&MaintainerTrustStore>,
) -> Result<(bool, bool, String), String> {
    let Some(signature) = entry.signature.as_ref() else {
        return Ok((false, false, "entry is unsigned".into()));
    };
    if signature.algorithm != "ed25519" {
        return Err(format!(
            "unsupported registry signature algorithm `{}`",
            signature.algorithm
        ));
    }
    let Some(store) = trust_store else {
        return Ok((
            false,
            false,
            format!(
                "signature by `{}` was not checked because no trust store was provided",
                signature.signer
            ),
        ));
    };
    let maintainer = store
        .maintainers
        .iter()
        .find(|maintainer| maintainer.id == signature.signer)
        .ok_or_else(|| format!("registry signer `{}` is not trusted", signature.signer))?;
    if maintainer.revoked {
        return Err(format!(
            "registry signer `{}` is revoked in the local trust store",
            signature.signer
        ));
    }
    let public_key: [u8; 32] = decode_hex_exact(&maintainer.public_key, 32)?
        .try_into()
        .map_err(|_| "invalid Ed25519 public key length".to_string())?;
    let verifying_key = VerifyingKey::from_bytes(&public_key)
        .map_err(|error| format!("invalid Ed25519 public key: {error}"))?;
    let signature_bytes: [u8; 64] = decode_hex_exact(&signature.value, 64)?
        .try_into()
        .map_err(|_| "invalid Ed25519 signature length".to_string())?;
    let signature_value = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(&signature_message(manifest, entry), &signature_value)
        .map_err(|_| format!("registry signature by `{}` is invalid", signature.signer))?;
    Ok((
        true,
        true,
        format!(
            "signature verified against pinned maintainer `{}`",
            signature.signer
        ),
    ))
}

fn write_cache(
    repository_root: &Path,
    final_root: &Path,
    manifest_raw: &str,
    content_root: &Path,
    verified: &VerifiedRegistryEntry,
    options: RegistryVerificationOptions<'_>,
    now_unix: u64,
) -> Result<(), String> {
    let canonical_repository = repository_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve repository root: {error}"))?;
    let final_relative = final_root
        .strip_prefix(repository_root)
        .map_err(|_| "cache target escaped repository root".to_string())?;
    reject_symlink_components(&canonical_repository, final_relative, false)?;
    if final_root.exists() {
        let receipt = final_root.join("cache-receipt.toml");
        if receipt.is_file() {
            let raw = fs::read_to_string(&receipt)
                .map_err(|error| format!("failed to read existing cache receipt: {error}"))?;
            let existing: CacheReceipt = toml::from_str(&raw)
                .map_err(|error| format!("invalid existing cache receipt: {error}"))?;
            verify_cached_entry(
                &canonical_repository.join(CACHE_ROOT),
                final_root,
                &existing,
                options.trust_store,
                options.now_unix,
            )?;
            return Ok(());
        }
        return Err(format!(
            "registry cache target `{}` already exists without a valid receipt",
            final_relative.display()
        ));
    }

    let cache_parent = canonical_repository.join(CACHE_ROOT);
    fs::create_dir_all(&cache_parent)
        .map_err(|error| format!("failed to create registry cache root: {error}"))?;
    reject_symlink_components(&canonical_repository, Path::new(CACHE_ROOT), false)?;
    let stage = cache_parent.join(format!(".stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage)
        .map_err(|error| format!("failed to create registry cache stage: {error}"))?;

    let write_result = (|| {
        let content_target = stage.join("content");
        for artifact in &verified.entry.artifacts {
            let relative = normalized_relative_path(&artifact.path)?;
            let source = content_root.join(&relative);
            let target = content_target.join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("failed to create registry cache directory: {error}")
                })?;
            }
            let bytes = fs::read(&source)
                .map_err(|error| format!("failed to read artifact `{}`: {error}", artifact.path))?;
            if sha256(&bytes) != artifact.sha256.to_ascii_lowercase() {
                return Err(format!(
                    "registry artifact `{}` changed after verification",
                    artifact.path
                ));
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|error| format!("failed to create cached artifact: {error}"))?;
            file.write_all(&bytes)
                .map_err(|error| format!("failed to write cached artifact: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync cached artifact: {error}"))?;
        }
        let mut manifest_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("registry-manifest.toml"))
            .map_err(|error| format!("failed to create cached registry manifest: {error}"))?;
        manifest_file
            .write_all(manifest_raw.as_bytes())
            .map_err(|error| format!("failed to write cached registry manifest: {error}"))?;
        manifest_file
            .sync_all()
            .map_err(|error| format!("failed to sync cached registry manifest: {error}"))?;
        let receipt = CacheReceipt {
            schema_version: REGISTRY_SCHEMA_VERSION,
            cached_at_unix: now_unix,
            registry_id: verified.registry_id.clone(),
            registry_version: verified.registry_version.clone(),
            entry_id: verified.entry.id.clone(),
            entry_version: verified.entry.version.clone(),
            manifest_sha256: verified.manifest_sha256.clone(),
            verified_at_unix: verified.entry.verified_at_unix,
            review_at_unix: verified.entry.review_at_unix,
            lifecycle: verified.entry.lifecycle,
            host: options.host.map(ToOwned::to_owned),
            artifacts: verified.entry.artifacts.clone(),
        };
        let raw = toml::to_string_pretty(&receipt)
            .map_err(|error| format!("failed to serialize cache receipt: {error}"))?;
        let mut receipt_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("cache-receipt.toml"))
            .map_err(|error| format!("failed to create cache receipt: {error}"))?;
        receipt_file
            .write_all(raw.as_bytes())
            .map_err(|error| format!("failed to write cache receipt: {error}"))?;
        receipt_file
            .sync_all()
            .map_err(|error| format!("failed to sync cache receipt: {error}"))?;
        Ok::<(), String>(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    if let Some(parent) = final_root.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create registry cache parent: {error}"))?;
    }
    if let Err(error) = fs::rename(&stage, final_root) {
        let _ = fs::remove_dir_all(&stage);
        return Err(format!(
            "failed to publish immutable registry cache: {error}"
        ));
    }
    Ok(())
}

fn collect_receipts(
    path: &Path,
    receipts: &mut Vec<(PathBuf, CacheReceipt)>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(path).map_err(|error| format!("failed to read registry cache: {error}"))?
    {
        let entry = entry.map_err(|error| format!("failed to read registry cache: {error}"))?;
        let metadata = entry
            .file_type()
            .map_err(|error| format!("failed to inspect registry cache: {error}"))?;
        if metadata.is_symlink() {
            return Err(format!(
                "registry cache contains forbidden symlink `{}`",
                entry.path().display()
            ));
        }
        if metadata.is_dir() {
            collect_receipts(&entry.path(), receipts)?;
        } else if entry.file_name() == "cache-receipt.toml" {
            let raw = fs::read_to_string(entry.path())
                .map_err(|error| format!("failed to read cache receipt: {error}"))?;
            let receipt: CacheReceipt =
                toml::from_str(&raw).map_err(|error| format!("invalid cache receipt: {error}"))?;
            receipts.push((path.to_path_buf(), receipt));
        }
    }
    Ok(())
}

fn verify_cached_entry(
    cache_root: &Path,
    receipt_root: &Path,
    receipt: &CacheReceipt,
    trust_store: Option<&MaintainerTrustStore>,
    now_unix: u64,
) -> Result<VerifiedRegistryEntry, String> {
    let relative = receipt_root
        .strip_prefix(cache_root)
        .map_err(|_| "cached registry entry is outside the cache root".to_string())?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if components.len() != 4
        || components[0] != receipt.registry_id
        || components[1] != receipt.entry_id
        || components[2] != receipt.entry_version
        || components[3] != receipt.manifest_sha256
    {
        return Err("cache receipt is not bound to its registry/id/version/hash path".into());
    }
    let manifest_raw = fs::read_to_string(receipt_root.join("registry-manifest.toml"))
        .map_err(|error| format!("failed to read cached registry manifest: {error}"))?;
    if sha256(manifest_raw.as_bytes()) != receipt.manifest_sha256 {
        return Err("cached registry manifest hash does not match its receipt/path".into());
    }
    let manifest = parse_manifest(&manifest_raw)?;
    if manifest.id != receipt.registry_id || manifest.version != receipt.registry_version {
        return Err("cached registry manifest identity does not match its receipt".into());
    }
    let entry = manifest
        .entries
        .iter()
        .find(|entry| entry.id == receipt.entry_id)
        .ok_or_else(|| "cached registry manifest does not contain the receipt entry".to_string())?;
    if entry.version != receipt.entry_version
        || entry.verified_at_unix != receipt.verified_at_unix
        || entry.review_at_unix != receipt.review_at_unix
        || entry.lifecycle != receipt.lifecycle
        || entry.artifacts.len() != receipt.artifacts.len()
        || entry
            .artifacts
            .iter()
            .zip(&receipt.artifacts)
            .any(|(manifest, receipt)| {
                manifest.path != receipt.path
                    || manifest.kind != receipt.kind
                    || manifest.sha256 != receipt.sha256
                    || manifest.size_bytes != receipt.size_bytes
            })
    {
        return Err("cache receipt metadata was changed from the anchored manifest".into());
    }
    verify_entry(
        &manifest_raw,
        &receipt.entry_id,
        &receipt_root.join("content"),
        trust_store,
        now_unix,
        env!("CARGO_PKG_VERSION"),
        receipt.host.as_deref(),
    )
}

fn load_cached_trust_store(repository_root: &Path) -> Result<Option<MaintainerTrustStore>, String> {
    let path = repository_root.join(TRUST_STORE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read cached registry trust store: {error}"))?;
    parse_trust_store(&raw).map(Some)
}

fn normalized_relative_path(raw: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if raw.is_empty() || raw.contains('\\') || path.is_absolute() {
        return Err(format!(
            "registry artifact path `{raw}` must be normalized, portable, and repository-relative"
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            _ => {
                return Err(format!(
                    "registry artifact path `{raw}` contains traversal or non-normal components"
                ));
            }
        }
    }
    if normalized.to_string_lossy().replace('\\', "/") != raw.replace('\\', "/") {
        return Err(format!("registry artifact path `{raw}` is not normalized"));
    }
    Ok(normalized)
}

fn reject_symlink_components(
    root: &Path,
    relative: &Path,
    require_leaf: bool,
) -> Result<(), String> {
    let mut cursor = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for component in &components {
        let Component::Normal(value) = component else {
            return Err("registry path contains a non-normal component".into());
        };
        cursor.push(value);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "registry path crosses forbidden symlink `{}`",
                    cursor.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if require_leaf {
                    return Err(format!(
                        "registry path `{}` does not exist",
                        cursor.display()
                    ));
                }
                break;
            }
            Err(error) => return Err(format!("failed to inspect registry path: {error}")),
        }
    }
    Ok(())
}

fn decode_hex_exact(raw: &str, expected_len: usize) -> Result<Vec<u8>, String> {
    if raw.len() != expected_len * 2 {
        return Err(format!(
            "expected {} hexadecimal characters",
            expected_len * 2
        ));
    }
    (0..raw.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&raw[index..index + 2], 16)
                .map_err(|_| "value is not lowercase or uppercase hexadecimal".to_string())
        })
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::TempDir;

    const TEST_NOW: u64 = 1_783_987_200;
    const TEST_REVIEW: u64 = 1_815_523_200;

    fn fixture(signature: Option<RegistrySignature>) -> (TempDir, RegistryManifest, String) {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("pack")).unwrap();
        fs::write(
            temp.path().join("pack/policy.toml"),
            include_str!("../presets/policies/balanced.toml"),
        )
        .unwrap();
        fs::write(
            temp.path().join("pack/binding.toml"),
            include_str!("../presets/bindings/codex-openai.toml"),
        )
        .unwrap();
        let report =
            crate::preset_eval::evaluate_embedded_suite(&crate::preset_eval::EvaluationOptions {
                at_unix: Some(TEST_NOW),
                ..Default::default()
            })
            .unwrap();
        fs::write(
            temp.path().join("pack/verification.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"report": report})).unwrap(),
        )
        .unwrap();
        let artifacts = [
            ("pack/policy.toml", RegistryArtifactKind::Policy),
            ("pack/binding.toml", RegistryArtifactKind::HostBinding),
            ("pack/verification.json", RegistryArtifactKind::Verification),
        ]
        .into_iter()
        .map(|(path, kind)| {
            let bytes = fs::read(temp.path().join(path)).unwrap();
            RegistryArtifact {
                path: path.into(),
                kind,
                sha256: sha256(&bytes),
                size_bytes: bytes.len() as u64,
            }
        })
        .collect();
        let manifest = RegistryManifest {
            schema_version: 1,
            id: "official".into(),
            version: "2026.07".into(),
            generated_at_unix: TEST_NOW,
            entries: vec![RegistryEntry {
                id: "balanced-codex".into(),
                version: "1.0.0".into(),
                kind: RegistryEntryKind::Pack,
                lifecycle: RegistryLifecycle::Published,
                verification_status: VerificationStatus::Recommended,
                verified_at_unix: TEST_NOW,
                review_at_unix: TEST_REVIEW,
                compatible_hosts: vec!["codex".into()],
                min_planr_version: Some("1.3.0".into()),
                max_planr_version: Some("1.9.0".into()),
                verification_path: "pack/verification.json".into(),
                evaluation: Some(RegistryEvaluationRef {
                    policy_id: "balanced".into(),
                    policy_version: "1.0.0".into(),
                    binding_id: "codex-openai".into(),
                    binding_version: "1.0.0".into(),
                    suite_id: "planr-preset-suite".into(),
                    suite_version: "1.8.0".into(),
                }),
                replacement: None,
                revocation_reason: None,
                artifacts,
                signature,
            }],
        };
        let raw = toml::to_string_pretty(&manifest).unwrap();
        (temp, manifest, raw)
    }

    fn replace_artifact(
        root: &TempDir,
        manifest: &mut RegistryManifest,
        path: &str,
        content: &str,
    ) -> String {
        fs::write(root.path().join(path), content).unwrap();
        let artifact = manifest.entries[0]
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.path == path)
            .unwrap();
        artifact.sha256 = sha256(content.as_bytes());
        artifact.size_bytes = content.len() as u64;
        toml::to_string_pretty(manifest).unwrap()
    }

    #[test]
    fn unsigned_current_entry_is_integrity_verified_but_not_recommended() {
        let (temp, _, raw) = fixture(None);
        let verified = verify_entry(
            &raw,
            "balanced-codex",
            temp.path(),
            None,
            TEST_NOW,
            "1.3.0",
            Some("codex"),
        )
        .unwrap();
        assert!(verified.integrity_verified);
        assert!(!verified.signature_verified);
        assert_eq!(verified.effective_status, "verified");
        assert!(!verified.recommended);
    }

    #[test]
    fn pinned_signature_cannot_recommend_without_evaluation_gates() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let (temp, mut manifest, _) = fixture(None);
        let signature = key.sign(&signature_message(&manifest, &manifest.entries[0]));
        manifest.entries[0].signature = Some(RegistrySignature {
            signer: "planr-maintainers".into(),
            algorithm: "ed25519".into(),
            value: signature
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        });
        let raw = toml::to_string_pretty(&manifest).unwrap();
        let store = MaintainerTrustStore {
            schema_version: 1,
            maintainers: vec![TrustedMaintainer {
                id: "planr-maintainers".into(),
                public_key: key
                    .verifying_key()
                    .to_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                revoked: false,
            }],
        };
        let verified = verify_entry(
            &raw,
            "balanced-codex",
            temp.path(),
            Some(&store),
            TEST_NOW,
            "1.3.0",
            Some("codex"),
        )
        .unwrap();
        assert!(verified.signature_verified);
        assert!(verified.trusted_maintainer);
        assert_eq!(verified.effective_status, "verified");
        assert!(!verified.recommended);
        assert!(verified.reasons.iter().any(|reason| {
            reason.contains("canonical evaluation report did not earn recommended")
        }));
    }

    #[test]
    fn tampering_incompatibility_and_revocation_fail_closed() {
        let (temp, mut manifest, raw) = fixture(None);
        fs::write(temp.path().join("pack/policy.toml"), "tampered").unwrap();
        assert!(
            verify_entry(
                &raw,
                "balanced-codex",
                temp.path(),
                None,
                TEST_NOW,
                "1.3.0",
                Some("codex")
            )
            .unwrap_err()
            .contains("mismatch")
        );

        let (temp, _, raw) = fixture(None);
        assert!(
            verify_entry(
                &raw,
                "balanced-codex",
                temp.path(),
                None,
                TEST_NOW,
                "2.0.0",
                Some("codex")
            )
            .unwrap_err()
            .contains("incompatible")
        );

        manifest.entries[0].lifecycle = RegistryLifecycle::Revoked;
        manifest.entries[0].revocation_reason = Some("compromised".into());
        let raw = toml::to_string_pretty(&manifest).unwrap();
        assert!(
            verify_entry(
                &raw,
                "balanced-codex",
                temp.path(),
                None,
                TEST_NOW,
                "1.3.0",
                Some("codex")
            )
            .unwrap_err()
            .contains("revoked")
        );
    }

    #[test]
    fn stale_or_deprecated_entries_are_visibly_demoted() {
        let (temp, mut manifest, _) = fixture(None);
        let raw = toml::to_string_pretty(&manifest).unwrap();
        let stale = verify_entry(
            &raw,
            "balanced-codex",
            temp.path(),
            None,
            TEST_REVIEW + 1,
            "1.3.0",
            Some("codex"),
        )
        .unwrap();
        assert_eq!(stale.effective_status, "stale");
        assert!(!stale.recommended);

        manifest.entries[0].lifecycle = RegistryLifecycle::Deprecated;
        manifest.entries[0].replacement = Some("balanced-codex@2".into());
        let raw = toml::to_string_pretty(&manifest).unwrap();
        let deprecated = verify_entry(
            &raw,
            "balanced-codex",
            temp.path(),
            None,
            TEST_NOW,
            "1.3.0",
            Some("codex"),
        )
        .unwrap();
        assert_eq!(deprecated.effective_status, "deprecated");
    }

    #[test]
    fn import_is_preview_first_content_minimized_and_offline_reusable() {
        let repository = TempDir::new().unwrap();
        let (content, _, raw) = fixture(None);
        fs::write(content.path().join("unlisted-secret"), "do not copy").unwrap();
        let options = RegistryVerificationOptions {
            trust_store: None,
            now_unix: TEST_NOW,
            planr_version: "1.3.0",
            host: Some("codex"),
        };
        let preview = import_entry(
            repository.path(),
            &raw,
            "balanced-codex",
            content.path(),
            options,
            false,
        )
        .unwrap();
        assert_eq!(preview.action, "preview");
        assert!(!repository.path().join(CACHE_ROOT).exists());

        let imported = import_entry(
            repository.path(),
            &raw,
            "balanced-codex",
            content.path(),
            options,
            true,
        )
        .unwrap();
        let cache = repository.path().join(&imported.cache_path);
        assert!(cache.join("content/pack/policy.toml").is_file());
        assert!(!cache.join("content/unlisted-secret").exists());
        fs::remove_dir_all(content.path()).unwrap();
        let listed = list_cache(repository.path(), TEST_REVIEW + 1).unwrap();
        assert_eq!(listed["entries"][0]["freshness"], "stale");
    }

    #[test]
    fn coordinated_content_and_receipt_tamper_cannot_redefine_cache_trust() {
        let repository = TempDir::new().unwrap();
        let (content, mut manifest, _) = fixture(None);
        let key = SigningKey::from_bytes(&[11; 32]);
        let signature = key.sign(&signature_message(&manifest, &manifest.entries[0]));
        manifest.entries[0].signature = Some(RegistrySignature {
            signer: "planr-maintainers".into(),
            algorithm: "ed25519".into(),
            value: signature
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        });
        let raw = toml::to_string_pretty(&manifest).unwrap();
        let public_key = key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let store = parse_trust_store(&format!(
            "schema_version = 1\n[[maintainers]]\nid = \"planr-maintainers\"\npublic_key = \"{public_key}\"\n"
        ))
        .unwrap();
        let trust_path = repository.path().join(TRUST_STORE_PATH);
        fs::create_dir_all(trust_path.parent().unwrap()).unwrap();
        fs::write(
            &trust_path,
            format!(
                "schema_version = 1\n[[maintainers]]\nid = \"planr-maintainers\"\npublic_key = \"{public_key}\"\n"
            ),
        )
        .unwrap();
        let imported = import_entry(
            repository.path(),
            &raw,
            "balanced-codex",
            content.path(),
            RegistryVerificationOptions {
                trust_store: Some(&store),
                now_unix: TEST_NOW,
                planr_version: "1.3.0",
                host: Some("codex"),
            },
            true,
        )
        .unwrap();
        let cache = repository.path().join(imported.cache_path);
        let before = list_cache(repository.path(), TEST_NOW).unwrap();
        assert_eq!(before["entries"][0]["signature_verified"], true);
        assert_eq!(before["entries"][0]["trusted_maintainer"], true);

        let tampered = "schema_version = 1\nid = \"attacker\"\nversion = \"9\"\n";
        fs::write(cache.join("content/pack/policy.toml"), tampered).unwrap();
        let receipt_path = cache.join("cache-receipt.toml");
        let mut receipt: CacheReceipt =
            toml::from_str(&fs::read_to_string(&receipt_path).unwrap()).unwrap();
        let policy = receipt
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.path == "pack/policy.toml")
            .unwrap();
        policy.sha256 = sha256(tampered.as_bytes());
        policy.size_bytes = tampered.len() as u64;
        fs::write(&receipt_path, toml::to_string_pretty(&receipt).unwrap()).unwrap();

        let after = list_cache(repository.path(), TEST_NOW).unwrap();
        assert_eq!(after["entries"][0]["integrity_verified"], false);
        assert_eq!(after["entries"][0]["usable"], false);
        assert_eq!(after["entries"][0]["signature_verified"], false);
        assert_eq!(after["entries"][0]["trusted_maintainer"], false);

        let mut attacker_receipt = fs::read_to_string(&receipt_path).unwrap();
        attacker_receipt.push_str("signature_verified = true\ntrusted_maintainer = true\n");
        fs::write(&receipt_path, attacker_receipt).unwrap();
        assert!(list_cache(repository.path(), TEST_NOW).is_err());
    }

    #[test]
    fn experimental_registry_policies_cannot_bypass_public_distribution_safety() {
        let repository = TempDir::new().unwrap();
        let (content, mut manifest, _) = fixture(None);
        manifest.entries[0].verification_status = VerificationStatus::Experimental;
        manifest.entries[0].evaluation = None;
        let mut policy =
            crate::usage_policy::parse_policy(include_str!("../presets/policies/balanced.toml"))
                .unwrap();
        let permissions = policy.execution.roles.get_mut("worker").unwrap();
        permissions
            .commands
            .insert(crate::execution_policy::CommandSpec {
                program: "cargo".into(),
                args: vec!["test".into()],
            });
        permissions.hooks.insert("pre_dispatch".into());
        permissions.network_hosts.insert("example.com".into());
        permissions.mcp_servers.insert("registry-tools".into());
        permissions
            .secret_references
            .insert("deploy-credential".into());
        permissions.filesystem.allow_overwrite = true;
        let raw = replace_artifact(
            &content,
            &mut manifest,
            "pack/policy.toml",
            &toml::to_string_pretty(&policy).unwrap(),
        );

        let error = verify_entry(
            &raw,
            "balanced-codex",
            content.path(),
            None,
            TEST_NOW,
            "1.3.0",
            Some("codex"),
        )
        .unwrap_err();
        for unsafe_grant in [
            "grants commands",
            "grants hooks",
            "references secrets",
            "grants network hosts",
            "grants MCP servers",
            "enables overwrite",
        ] {
            assert!(
                error.contains(unsafe_grant),
                "missing `{unsafe_grant}`: {error}"
            );
        }
        assert!(
            import_entry(
                repository.path(),
                &raw,
                "balanced-codex",
                content.path(),
                RegistryVerificationOptions {
                    trust_store: None,
                    now_unix: TEST_NOW,
                    planr_version: "1.3.0",
                    host: Some("codex"),
                },
                true,
            )
            .is_err()
        );
        assert!(!repository.path().join(CACHE_ROOT).exists());
    }

    #[test]
    fn registry_metadata_rejects_secrets_without_echoing_them() {
        let secret = "sk-registry-secret-value";
        for field in [
            "registry_version",
            "entry_version",
            "compatible_host",
            "minimum_version",
            "replacement",
            "revocation_reason",
            "evaluation_version",
            "artifact_path",
            "signature_signer",
        ] {
            let (_, mut manifest, _) = fixture(None);
            match field {
                "registry_version" => manifest.version = secret.into(),
                "entry_version" => manifest.entries[0].version = secret.into(),
                "compatible_host" => manifest.entries[0].compatible_hosts = vec![secret.into()],
                "minimum_version" => manifest.entries[0].min_planr_version = Some(secret.into()),
                "replacement" => manifest.entries[0].replacement = Some(secret.into()),
                "revocation_reason" => manifest.entries[0].revocation_reason = Some(secret.into()),
                "evaluation_version" => {
                    manifest.entries[0]
                        .evaluation
                        .as_mut()
                        .unwrap()
                        .suite_version = secret.into()
                }
                "artifact_path" => manifest.entries[0].artifacts[0].path = secret.into(),
                "signature_signer" => {
                    manifest.entries[0].signature = Some(RegistrySignature {
                        signer: secret.into(),
                        algorithm: "ed25519".into(),
                        value: "00".repeat(64),
                    })
                }
                _ => unreachable!(),
            }
            let error = parse_manifest(&toml::to_string_pretty(&manifest).unwrap()).unwrap_err();
            assert!(error.contains("secret-like"), "field `{field}`: {error}");
            assert!(!error.contains(secret), "field `{field}` leaked metadata");
        }
    }

    #[test]
    fn registry_signers_and_replacements_are_bounded_identifiers() {
        let (_, mut manifest, _) = fixture(None);
        manifest.entries[0].replacement = Some("balanced-codex/2".into());
        assert!(
            parse_manifest(&toml::to_string_pretty(&manifest).unwrap())
                .unwrap_err()
                .contains("ASCII letters")
        );

        let (_, mut manifest, _) = fixture(None);
        manifest.entries[0].signature = Some(RegistrySignature {
            signer: "maintainers/planr".into(),
            algorithm: "ed25519".into(),
            value: "00".repeat(64),
        });
        assert!(
            parse_manifest(&toml::to_string_pretty(&manifest).unwrap())
                .unwrap_err()
                .contains("signature signer")
        );
    }

    #[test]
    fn registry_host_bindings_reuse_canonical_semantic_and_safe_artifact_validation() {
        let (content, mut manifest, _) = fixture(None);
        let mut binding = crate::preset::parse_host_binding(include_str!(
            "../presets/bindings/codex-openai.toml"
        ))
        .unwrap();
        binding.artifacts[0].path = ".codex/config.toml".into();
        let raw = replace_artifact(
            &content,
            &mut manifest,
            "pack/binding.toml",
            &toml::to_string_pretty(&binding).unwrap(),
        );
        assert!(
            verify_entry(
                &raw,
                "balanced-codex",
                content.path(),
                None,
                TEST_NOW,
                "1.3.0",
                Some("codex")
            )
            .unwrap_err()
            .contains("outside the agent-role surface")
        );

        let (content, mut manifest, _) = fixture(None);
        let mut binding = crate::preset::parse_host_binding(include_str!(
            "../presets/bindings/codex-openai.toml"
        ))
        .unwrap();
        binding.billing_assumptions.push("sk-secret-token".into());
        let raw = replace_artifact(
            &content,
            &mut manifest,
            "pack/binding.toml",
            &toml::to_string_pretty(&binding).unwrap(),
        );
        assert!(
            verify_entry(
                &raw,
                "balanced-codex",
                content.path(),
                None,
                TEST_NOW,
                "1.3.0",
                Some("codex")
            )
            .unwrap_err()
            .contains("secret-like")
        );

        let (content, mut manifest, _) = fixture(None);
        let mut binding = crate::preset::parse_host_binding(include_str!(
            "../presets/bindings/codex-openai.toml"
        ))
        .unwrap();
        binding.routes[0].role = "missing-role".into();
        let raw = replace_artifact(
            &content,
            &mut manifest,
            "pack/binding.toml",
            &toml::to_string_pretty(&binding).unwrap(),
        );
        let error = verify_entry(
            &raw,
            "balanced-codex",
            content.path(),
            None,
            TEST_NOW,
            "1.3.0",
            Some("codex"),
        )
        .unwrap_err();
        assert!(error.contains("missing-role") || error.contains("unknown role"));
    }

    #[cfg(unix)]
    #[test]
    fn source_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let (content, _, raw) = fixture(None);
        let outside = TempDir::new().unwrap();
        fs::remove_file(content.path().join("pack/policy.toml")).unwrap();
        fs::write(outside.path().join("policy.toml"), "schema_version = 1\n").unwrap();
        symlink(
            outside.path().join("policy.toml"),
            content.path().join("pack/policy.toml"),
        )
        .unwrap();
        assert!(
            verify_entry(
                &raw,
                "balanced-codex",
                content.path(),
                None,
                TEST_NOW,
                "1.3.0",
                Some("codex")
            )
            .unwrap_err()
            .contains("symlink")
        );
    }
}
