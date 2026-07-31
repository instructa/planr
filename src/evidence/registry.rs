#![allow(dead_code)]

use super::adapter_signal::{AdapterBoundarySignal, adapter_boundary_signal_from_process_output};
use super::model::{
    AttemptStatus, CapabilityAvailability, CapabilityAvailabilityStatus, EnvironmentBinding,
    EvidenceId, ObservedPayloadContract, PayloadSchemaBinding, PermissionState, ProbeCheck,
    ProbeResult, ProcessExecutionContract, ProvenanceSourceKind, SchemaVersion, Sha256Digest,
    VerificationCapabilityInstance, VerificationCapabilityManifest,
};
use crate::canonical_json::{sha256_json_digest, sha256_prefixed_bytes};
use crate::execution::{BoundedProcessInput, CancellationToken, run_bounded_process};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilityRegistryDiagnostic {
    pub manifest_id: String,
    pub code: CapabilityRegistryDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityRegistryDiagnosticCode {
    DeclaredManifestUnavailable,
    DeclaredManifestMismatch,
    DuplicateManifestId,
    InvalidRepositoryRoot,
    UnsafeManifestPath,
    UnsafeExecutionContract,
    ProbeUnavailable,
    PermissionDenied,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityRegistry {
    repository_root: PathBuf,
    repository_root_digest: String,
    capabilities: BTreeMap<String, RegisteredCapability>,
    runtime_availability: BTreeMap<RuntimeAvailabilityKey, CapabilityAvailabilityStatus>,
    runtime_diagnostics: BTreeMap<RuntimeAvailabilityKey, CapabilityRegistryDiagnostic>,
    diagnostics: Vec<CapabilityRegistryDiagnostic>,
    fatal_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityInstanceResolution {
    pub instance: VerificationCapabilityInstance,
    pub reused: bool,
    pub reason: CapabilityInstanceResolutionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityInstanceResolutionReason {
    ReusedCurrent,
    ProbedNoCurrent,
    ReprobedExpired,
    ReprobedRuntimeMismatch,
    ReprobedEnvironmentMismatch,
    ReprobedRegistrationMismatch,
}

impl CapabilityInstanceResolutionReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReusedCurrent => "reused_current",
            Self::ProbedNoCurrent => "probed_no_current",
            Self::ReprobedExpired => "reprobed_expired",
            Self::ReprobedRuntimeMismatch => "reprobed_runtime_mismatch",
            Self::ReprobedEnvironmentMismatch => "reprobed_environment_mismatch",
            Self::ReprobedRegistrationMismatch => "reprobed_registration_mismatch",
        }
    }
}

impl CapabilityRegistry {
    pub(crate) fn from_manifests_and_adapter_registrations(
        repository_root: &Path,
        builtins: impl IntoIterator<Item = VerificationCapabilityManifest>,
        adapter_registrations: &[Value],
    ) -> Self {
        let repository_root = match canonical_repository_root(repository_root) {
            Ok(repository_root) => repository_root,
            Err(error) => return Self::fatal_invalid_repository_root(repository_root, error),
        };
        let repository_root_digest = repository_root_digest(&repository_root);
        let mut registry = Self {
            repository_root,
            repository_root_digest,
            capabilities: BTreeMap::new(),
            runtime_availability: BTreeMap::new(),
            runtime_diagnostics: BTreeMap::new(),
            diagnostics: Vec::new(),
            fatal_error: None,
        };
        for manifest in builtins {
            registry.register_manifest(CapabilitySource::BuiltIn, None, None, manifest);
        }
        for value in adapter_registrations {
            registry.register_repository_adapter(value);
        }
        registry
    }

    fn fatal_invalid_repository_root(repository_root: &Path, error: anyhow::Error) -> Self {
        let message = format!(
            "repository root {} must canonicalize to an existing directory: {error}",
            repository_root.display()
        );
        Self {
            repository_root: PathBuf::new(),
            repository_root_digest: "invalid-repository-root".to_string(),
            capabilities: BTreeMap::new(),
            runtime_availability: BTreeMap::new(),
            runtime_diagnostics: BTreeMap::new(),
            diagnostics: vec![CapabilityRegistryDiagnostic {
                manifest_id: "repository-root".to_string(),
                code: CapabilityRegistryDiagnosticCode::InvalidRepositoryRoot,
                message: message.clone(),
            }],
            fatal_error: Some(message),
        }
    }

    pub(crate) fn capabilities(&self) -> impl Iterator<Item = &RegisteredCapability> {
        self.capabilities.values()
    }

    pub(crate) fn diagnostics(&self) -> Vec<CapabilityRegistryDiagnostic> {
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.extend(self.runtime_diagnostics.values().cloned());
        diagnostics
    }

    pub(crate) fn available_diagnostics_for_declared_observations(
        &self,
        declared_observation_types: impl IntoIterator<Item = String>,
    ) -> Vec<CapabilityRegistryDiagnostic> {
        declared_observation_types
            .into_iter()
            .filter_map(|observation_type| {
                let candidates = self
                    .capabilities
                    .values()
                    .filter(|capability| {
                        capability
                            .manifest
                            .supported_observations
                            .iter()
                            .any(|binding| binding.observation_type.as_str() == observation_type)
                    })
                    .collect::<Vec<_>>();
                if candidates.is_empty() {
                    return Some(CapabilityRegistryDiagnostic {
                        manifest_id: observation_type.clone(),
                        code: CapabilityRegistryDiagnosticCode::DeclaredManifestUnavailable,
                        message: format!(
                            "declared observation type {observation_type} has no registered capability"
                        ),
                    });
                }
                if candidates.iter().any(|capability| {
                    self.runtime_availability.iter().any(|(key, status)| {
                        key.matches_capability(capability)
                            && *status == CapabilityAvailabilityStatus::Available
                    })
                }) {
                    return None;
                }
                if candidates.iter().any(|capability| {
                    self.runtime_availability.iter().any(|(key, status)| {
                        key.matches_capability(capability)
                            && *status == CapabilityAvailabilityStatus::PermissionDenied
                    })
                }) {
                    return Some(CapabilityRegistryDiagnostic {
                        manifest_id: observation_type.clone(),
                        code: CapabilityRegistryDiagnosticCode::PermissionDenied,
                        message: format!(
                            "declared observation type {observation_type} has only permission-denied capability instances"
                        ),
                    });
                }
                if candidates.iter().any(|capability| {
                    self.runtime_availability
                        .keys()
                        .any(|key| key.matches_capability(capability))
                }) {
                    return Some(CapabilityRegistryDiagnostic {
                        manifest_id: observation_type.clone(),
                        code: CapabilityRegistryDiagnosticCode::ProbeUnavailable,
                        message: format!(
                            "declared observation type {observation_type} has no currently available capability instance"
                        ),
                    });
                }
                Some(CapabilityRegistryDiagnostic {
                    manifest_id: observation_type.clone(),
                    code: CapabilityRegistryDiagnosticCode::DeclaredManifestUnavailable,
                    message: format!(
                        "declared observation type {observation_type} has registered capabilities but no runtime probe"
                    ),
                })
            })
            .collect()
    }

    pub(crate) fn probe_and_store(
        &mut self,
        conn: &Connection,
        repository_root: &Path,
        manifest_id: &str,
        runtime: CapabilityRuntimeContext<'_>,
    ) -> Result<VerificationCapabilityInstance> {
        if let Some(error) = &self.fatal_error {
            bail!("{error}");
        }
        let runtime = ValidatedCapabilityRuntimeContext::parse(runtime)?;
        self.validate_probe_repository_root(repository_root)?;
        let capability = self
            .capabilities
            .get(manifest_id)
            .with_context(|| format!("capability manifest {manifest_id} is not registered"))?;
        let command_resolution =
            ProbeCommandResolution::capture(&capability.manifest.availability_probe.execution);
        store_capability_manifest(conn, capability)?;
        let now = timestamp();
        let probe_execution_id = probe_execution_id(
            capability,
            &runtime,
            &self.repository_root_digest,
            &command_resolution,
            &now,
        )?;
        let runtime_key = RuntimeAvailabilityKey::new(
            capability,
            &runtime,
            &self.repository_root_digest,
            &command_resolution,
        )?;
        let probe = if capability
            .manifest
            .supported_surfaces
            .iter()
            .any(|surface| surface.as_str() == runtime.surface.as_str())
        {
            run_process_probe(
                &self.repository_root,
                &capability.manifest.availability_probe.execution,
                &command_resolution,
                probe_execution_id,
                &now,
            )
        } else {
            failed_probe(
                probe_execution_id,
                now.clone(),
                format!(
                    "runtime surface {} is not declared by capability manifest {}",
                    runtime.surface,
                    capability.manifest.id.as_str()
                ),
                AttemptStatus::Skipped,
                CapabilityAvailabilityStatus::Unsupported,
            )
        };
        let runtime_diagnostic = (probe.availability.status
            != CapabilityAvailabilityStatus::Available)
            .then(|| CapabilityRegistryDiagnostic {
                manifest_id: manifest_id.to_string(),
                code: diagnostic_code_for_availability(probe.availability.status),
                message: probe
                    .availability
                    .reason
                    .clone()
                    .unwrap_or_else(|| "capability probe did not pass".to_string()),
            });

        let instance = capability_instance(
            capability,
            &runtime,
            &self.repository_root_digest,
            &command_resolution,
            probe,
            now,
        )?;
        store_capability_instance(conn, &instance, capability)?;
        self.runtime_availability
            .insert(runtime_key.clone(), instance.availability.status);
        if let Some(diagnostic) = runtime_diagnostic {
            self.runtime_diagnostics.insert(runtime_key, diagnostic);
        } else {
            self.runtime_diagnostics.remove(&runtime_key);
        }
        Ok(instance)
    }

    pub(crate) fn current_or_probe_and_store(
        &mut self,
        conn: &Connection,
        repository_root: &Path,
        manifest_id: &str,
        runtime: CapabilityRuntimeContext<'_>,
    ) -> Result<CapabilityInstanceResolution> {
        if let Some(error) = &self.fatal_error {
            bail!("{error}");
        }
        let runtime = ValidatedCapabilityRuntimeContext::parse(runtime)?;
        self.validate_probe_repository_root(repository_root)?;
        let capability = self
            .capabilities
            .get(manifest_id)
            .with_context(|| format!("capability manifest {manifest_id} is not registered"))?;
        let command_resolution =
            ProbeCommandResolution::capture(&capability.manifest.availability_probe.execution);
        store_capability_manifest(conn, capability)?;
        let runtime_key = RuntimeAvailabilityKey::new(
            capability,
            &runtime,
            &self.repository_root_digest,
            &command_resolution,
        )?;
        let current = current_compatible_capability_instance(
            conn,
            capability,
            &runtime,
            &self.repository_root_digest,
            &command_resolution,
        )?;
        if let Some(instance) = current.instance {
            self.runtime_availability
                .insert(runtime_key.clone(), instance.availability.status);
            if instance.availability.status == CapabilityAvailabilityStatus::Available {
                self.runtime_diagnostics.remove(&runtime_key);
            }
            return Ok(CapabilityInstanceResolution {
                instance,
                reused: true,
                reason: CapabilityInstanceResolutionReason::ReusedCurrent,
            });
        }
        let instance = self.probe_and_store(
            conn,
            repository_root,
            manifest_id,
            CapabilityRuntimeContext {
                host: runtime.host.as_str(),
                surface: runtime.surface.as_str(),
                host_version: runtime.host_version.as_str(),
                environment_id: runtime.environment_id.as_str(),
            },
        )?;
        Ok(CapabilityInstanceResolution {
            instance,
            reused: false,
            reason: current.miss_reason,
        })
    }

    pub(crate) fn store_verified_host_capture_instance(
        &mut self,
        conn: &Connection,
        manifest: VerificationCapabilityManifest,
        instance: VerificationCapabilityInstance,
    ) -> Result<()> {
        self.store_verified_host_capture_instance_with_expiry(conn, manifest, instance, None)
    }

    pub(crate) fn store_verified_host_capture_instance_with_expiry(
        &mut self,
        conn: &Connection,
        manifest: VerificationCapabilityManifest,
        instance: VerificationCapabilityInstance,
        valid_until: Option<&str>,
    ) -> Result<()> {
        self.register_manifest(CapabilitySource::BuiltIn, None, None, manifest);
        let capability = self
            .capabilities
            .get(instance.manifest_id.as_str())
            .with_context(|| {
                format!(
                    "host capability manifest {} was not registered",
                    instance.manifest_id.as_str()
                )
            })?;
        if capability.manifest_digest != instance.manifest_digest.as_str() {
            bail!(
                "host capability instance {} manifest digest {} does not match registered manifest {}",
                instance.id.as_str(),
                instance.manifest_digest.as_str(),
                capability.manifest_digest
            );
        }
        store_capability_manifest(conn, capability)?;
        store_capability_instance_with_expiry(conn, &instance, capability, valid_until)
    }

    fn validate_probe_repository_root(&self, repository_root: &Path) -> Result<()> {
        let supplied = canonical_repository_root(repository_root)?;
        if supplied == self.repository_root {
            Ok(())
        } else {
            bail!(
                "probe repository root {} does not match bound registry root {}",
                supplied.display(),
                self.repository_root.display()
            );
        }
    }

    fn register_repository_adapter(&mut self, value: &Value) {
        let declaration =
            match serde_json::from_value::<RepositoryAdapterRegistration>(value.clone()) {
                Ok(declaration) => declaration,
                Err(error) => {
                    self.diagnostics.push(CapabilityRegistryDiagnostic {
                        manifest_id: "<unknown>".to_string(),
                        code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                        message: format!(
                            "adapter registration does not match registry schema: {error}"
                        ),
                    });
                    return;
                }
            };
        if let Err(error) = validate_repository_relative_path(&declaration.manifest_path) {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id: declaration.manifest_id,
                code: CapabilityRegistryDiagnosticCode::UnsafeManifestPath,
                message: error.to_string(),
            });
            return;
        }
        if let Err(error) = validate_process_execution_contract(&declaration.execution_contract) {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id: declaration.manifest_id,
                code: CapabilityRegistryDiagnosticCode::UnsafeExecutionContract,
                message: error.to_string(),
            });
            return;
        }

        let manifest_path =
            match contained_repository_path(&self.repository_root, &declaration.manifest_path) {
                Ok(path) => path,
                Err(error) => {
                    self.diagnostics.push(CapabilityRegistryDiagnostic {
                        manifest_id: declaration.manifest_id,
                        code: CapabilityRegistryDiagnosticCode::UnsafeManifestPath,
                        message: error.to_string(),
                    });
                    return;
                }
            };
        let manifest_value = match fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading manifest {}", manifest_path.display()))
            .and_then(|text| serde_json::from_str::<Value>(&text).context("parsing manifest JSON"))
        {
            Ok(value) => value,
            Err(error) => {
                self.diagnostics.push(CapabilityRegistryDiagnostic {
                    manifest_id: declaration.manifest_id,
                    code: CapabilityRegistryDiagnosticCode::DeclaredManifestUnavailable,
                    message: error.to_string(),
                });
                return;
            }
        };
        let actual_digest = match sha256_json_digest(&manifest_value) {
            Ok(digest) => digest,
            Err(error) => {
                self.diagnostics.push(CapabilityRegistryDiagnostic {
                    manifest_id: declaration.manifest_id,
                    code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                    message: error.to_string(),
                });
                return;
            }
        };
        let manifest =
            match serde_json::from_value::<VerificationCapabilityManifest>(manifest_value) {
                Ok(manifest) => manifest,
                Err(error) => {
                    self.diagnostics.push(CapabilityRegistryDiagnostic {
                        manifest_id: declaration.manifest_id,
                        code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                        message: error.to_string(),
                    });
                    return;
                }
            };
        let mut mismatches = Vec::new();
        if declaration.manifest_id != manifest.id.as_str() {
            mismatches.push(format!(
                "manifest_id declared {}, manifest contains {}",
                declaration.manifest_id,
                manifest.id.as_str()
            ));
        }
        if declaration.manifest_digest != actual_digest {
            mismatches.push(format!(
                "manifest_digest declared {}, actual {}",
                declaration.manifest_digest, actual_digest
            ));
        }
        if declaration.provenance_path != manifest.provenance_path {
            mismatches.push("provenance_path does not match manifest".to_string());
        }
        if declaration.observation_types != observation_type_set(&manifest.supported_observations) {
            mismatches
                .push("observation_types do not match manifest supported_observations".to_string());
        }
        if serde_json::to_value(&declaration.payload_schemas).ok()
            != serde_json::to_value(&manifest.supported_observations).ok()
        {
            mismatches
                .push("payload_schemas do not match manifest supported_observations".to_string());
        }
        if !payload_schema_matches(
            &declaration.execution_contract.payload_schema,
            &manifest.supported_observations,
        ) {
            mismatches.push(
                "execution_contract payload_schema does not match manifest supported_observations"
                    .to_string(),
            );
        }
        if !mismatches.is_empty() {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id: declaration.manifest_id,
                code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                message: mismatches.join("; "),
            });
            return;
        }

        self.register_manifest(
            CapabilitySource::Repository {
                manifest_path: declaration.manifest_path,
            },
            Some(actual_digest),
            Some(declaration.execution_contract),
            manifest,
        );
    }

    fn register_manifest(
        &mut self,
        source: CapabilitySource,
        computed_manifest_digest: Option<String>,
        repository_execution_contract: Option<ProcessExecutionContract>,
        manifest: VerificationCapabilityManifest,
    ) {
        let manifest_id = manifest.id.as_str().to_string();
        if let Some(existing) = self.capabilities.get(&manifest_id) {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id,
                code: CapabilityRegistryDiagnosticCode::DuplicateManifestId,
                message: format!(
                    "capability manifest id is already registered from {}",
                    existing.source.label()
                ),
            });
            return;
        }
        if let Err(error) = validate_manifest_runtime_permissions(&manifest) {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id,
                code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                message: error.to_string(),
            });
            return;
        }
        if let Err(error) =
            validate_process_execution_contract(&manifest.availability_probe.execution)
        {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id,
                code: CapabilityRegistryDiagnosticCode::UnsafeExecutionContract,
                message: error.to_string(),
            });
            return;
        }
        if let Err(error) = validate_manifest_payload_projection(&manifest) {
            self.diagnostics.push(CapabilityRegistryDiagnostic {
                manifest_id,
                code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                message: error.to_string(),
            });
            return;
        }
        let manifest_digest = match computed_manifest_digest {
            Some(digest) => digest,
            None => match serde_json::to_value(&manifest)
                .context("serializing verification capability manifest")
                .and_then(|value| sha256_json_digest(&value))
            {
                Ok(digest) => digest,
                Err(error) => {
                    self.diagnostics.push(CapabilityRegistryDiagnostic {
                        manifest_id,
                        code: CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch,
                        message: format!("failed to compute canonical manifest digest: {error}"),
                    });
                    return;
                }
            },
        };
        self.capabilities.insert(
            manifest_id,
            RegisteredCapability {
                manifest_digest,
                repository_execution_contract,
                source,
                manifest,
            },
        );
    }
}

fn validate_manifest_runtime_permissions(manifest: &VerificationCapabilityManifest) -> Result<()> {
    manifest_permission_state(manifest).map(|_| ())
}

fn validate_manifest_payload_projection(manifest: &VerificationCapabilityManifest) -> Result<()> {
    if payload_schema_matches(
        &manifest.availability_probe.execution.payload_schema,
        &manifest.supported_observations,
    ) {
        Ok(())
    } else {
        bail!(
            "availability_probe.execution payload_schema does not match manifest supported_observations"
        );
    }
}

fn required_non_empty_manifest_permission(
    manifest: &VerificationCapabilityManifest,
    name: &'static str,
) -> Result<()> {
    let field = format!("permissions.{name}");
    match manifest.permissions.get(name).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(()),
        _ => bail!("{field} is required before capability registration"),
    }
}

fn optional_non_empty_manifest_permission(
    manifest: &VerificationCapabilityManifest,
    name: &'static str,
) -> Result<Option<String>> {
    let Some(value) = manifest.permissions.get(name) else {
        return Ok(None);
    };
    let field = format!("permissions.{name}");
    match value.as_str() {
        Some(value) if !value.is_empty() => Ok(Some(value.to_string())),
        _ => bail!("{field} must be a non-empty string when declared"),
    }
}

fn manifest_permission_state(manifest: &VerificationCapabilityManifest) -> Result<PermissionState> {
    let permission = |name| -> Result<String> {
        let field = format!("permissions.{name}");
        match manifest.permissions.get(name).and_then(Value::as_str) {
            Some(value) if !value.is_empty() => Ok(value.to_string()),
            _ => bail!("{field} is required before capability registration"),
        }
    };
    Ok(PermissionState {
        network: permission("network")?,
        filesystem: permission("filesystem")?,
        environment: optional_non_empty_manifest_permission(manifest, "environment")?,
        secrets: optional_non_empty_manifest_permission(manifest, "secrets")?,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredCapability {
    pub source: CapabilitySource,
    pub manifest_digest: String,
    pub repository_execution_contract: Option<ProcessExecutionContract>,
    pub manifest: VerificationCapabilityManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimeAvailabilityKey {
    manifest_id: String,
    manifest_digest: String,
    manifest_version: String,
    environment_id: String,
    permission_digest: String,
}

impl RuntimeAvailabilityKey {
    fn new(
        capability: &RegisteredCapability,
        runtime: &ValidatedCapabilityRuntimeContext,
        repository_root_digest: &str,
        command_resolution: &ProbeCommandResolution,
    ) -> Result<Self> {
        Ok(Self {
            manifest_id: capability.manifest.id.as_str().to_string(),
            manifest_digest: capability.manifest_digest.clone(),
            manifest_version: capability.manifest.version.clone(),
            environment_id: runtime.environment_id.as_str().to_string(),
            permission_digest: sha256_json_digest(&json!({
                "permissions": capability.manifest.permissions,
                "host": runtime.host,
                "surface": runtime.surface,
                "host_version": runtime.host_version,
                "repository_root": repository_root_digest,
                "command_resolution": command_resolution.identity(),
            }))?,
        })
    }

    fn matches_capability(&self, capability: &RegisteredCapability) -> bool {
        self.manifest_id == capability.manifest.id.as_str()
            && self.manifest_digest == capability.manifest_digest
            && self.manifest_version == capability.manifest.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CapabilitySource {
    BuiltIn,
    Repository { manifest_path: String },
}

impl CapabilitySource {
    fn label(&self) -> &'static str {
        match self {
            Self::BuiltIn => "built-in",
            Self::Repository { .. } => "repository",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CapabilityRuntimeContext<'a> {
    pub host: &'a str,
    pub surface: &'a str,
    pub host_version: &'a str,
    pub environment_id: &'a str,
}

#[derive(Debug, Clone)]
struct ValidatedCapabilityRuntimeContext {
    host: String,
    surface: String,
    host_version: String,
    environment_id: EvidenceId,
}

impl ValidatedCapabilityRuntimeContext {
    fn parse(runtime: CapabilityRuntimeContext<'_>) -> Result<Self> {
        if runtime.host.is_empty() {
            bail!("runtime host must be non-empty");
        }
        if runtime.surface.is_empty() {
            bail!("runtime surface must be non-empty");
        }
        if runtime.host_version.is_empty() {
            bail!("runtime host_version must be non-empty");
        }
        Ok(Self {
            host: runtime.host.to_string(),
            surface: runtime.surface.to_string(),
            host_version: runtime.host_version.to_string(),
            environment_id: EvidenceId::parse(runtime.environment_id.to_string())
                .context("runtime environment_id must be a valid EvidenceId")?,
        })
    }
}

#[derive(Debug)]
struct ProbeOutcome {
    availability: CapabilityAvailability,
    result: ProbeResult,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryAdapterRegistration {
    manifest_id: String,
    manifest_path: String,
    manifest_digest: String,
    observation_types: BTreeSet<String>,
    payload_schemas: Vec<PayloadSchemaBinding>,
    provenance_path: ProvenanceSourceKind,
    execution_contract: ProcessExecutionContract,
}

fn observation_type_set(bindings: &[PayloadSchemaBinding]) -> BTreeSet<String> {
    bindings
        .iter()
        .map(|binding| binding.observation_type.as_str().to_string())
        .collect()
}

fn validate_process_execution_contract(execution: &ProcessExecutionContract) -> Result<()> {
    if execution.kind != "process" {
        bail!("execution contract kind must be process");
    }
    validate_executable_name(&execution.executable)?;
    if let Some(working_directory) = &execution.working_directory {
        validate_repository_relative_path(working_directory)?;
    }
    if execution.timeout_ms == 0
        || execution.stdout_limit_bytes == 0
        || execution.stderr_limit_bytes == 0
    {
        bail!("process probe must declare non-zero timeout and output limits");
    }
    Ok(())
}

fn payload_schema_matches(
    candidate: &PayloadSchemaBinding,
    supported: &[PayloadSchemaBinding],
) -> bool {
    supported.iter().any(|binding| {
        binding.observation_type.as_str() == candidate.observation_type.as_str()
            && binding.schema_ref == candidate.schema_ref
            && binding.schema_digest.as_str() == candidate.schema_digest.as_str()
    })
}

fn validate_executable_name(executable: &str) -> Result<()> {
    if executable.is_empty()
        || executable.contains('/')
        || executable.contains('\\')
        || Path::new(executable).is_absolute()
    {
        bail!("process executable must be a bare command name resolved by PATH");
    }
    Ok(())
}

fn validate_repository_relative_path(path: &str) -> Result<()> {
    if path.is_empty() {
        bail!("repository path must be non-empty");
    }
    let path = Path::new(path);
    if path.is_absolute() {
        bail!("repository path must be relative");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("repository path must stay within the repository root");
            }
        }
    }
    Ok(())
}

fn contained_repository_path(repository_root: &Path, relative: &str) -> Result<PathBuf> {
    validate_repository_relative_path(relative)?;
    let canonical_root = repository_root.canonicalize().with_context(|| {
        format!(
            "canonicalizing repository root {}",
            repository_root.display()
        )
    })?;
    let path = canonical_root.join(relative);
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("canonicalizing repository path {}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("repository path escapes repository root");
    }
    Ok(canonical_path)
}

fn canonical_repository_root(repository_root: &Path) -> Result<PathBuf> {
    let canonical_root = repository_root.canonicalize().with_context(|| {
        format!(
            "canonicalizing repository root {}",
            repository_root.display()
        )
    })?;
    if !canonical_root.is_dir() {
        bail!(
            "repository root {} must be a directory",
            canonical_root.display()
        );
    }
    Ok(canonical_root)
}

fn repository_root_digest(repository_root: &Path) -> String {
    sha256_prefixed_bytes(repository_root.to_string_lossy().as_bytes())
}

#[cfg(test)]
thread_local! {
    static TEST_PATH_ENV: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn captured_path_env() -> Option<String> {
    #[cfg(test)]
    {
        if let Some(path) = TEST_PATH_ENV.with(|path| path.borrow().clone()) {
            return Some(path);
        }
    }
    env::var("PATH").ok()
}

#[cfg(test)]
fn with_captured_path_env_override<T>(path: String, f: impl FnOnce() -> T) -> T {
    let previous = TEST_PATH_ENV.with(|slot| slot.replace(Some(path)));
    let result = f();
    TEST_PATH_ENV.with(|slot| slot.replace(previous));
    result
}

#[derive(Debug, Clone)]
struct ProbeCommandResolution {
    executable: String,
    path_value: Option<String>,
    path_digest: String,
    resolved: Option<ResolvedProbeExecutable>,
    error: Option<String>,
}

impl ProbeCommandResolution {
    fn capture(execution: &ProcessExecutionContract) -> Self {
        let path = captured_path_env();
        let path_digest = sha256_prefixed_bytes(path.as_deref().unwrap_or("").as_bytes());
        match resolve_probe_executable(&execution.executable, path.as_deref()) {
            Ok(resolved) => Self {
                executable: execution.executable.clone(),
                path_value: path,
                path_digest,
                resolved: Some(resolved),
                error: None,
            },
            Err(error) => Self {
                executable: execution.executable.clone(),
                path_value: path,
                path_digest,
                resolved: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn identity(&self) -> Value {
        json!({
            "executable": self.executable,
            "path_digest": self.path_digest,
            "resolved": self.resolved.as_ref().map(ResolvedProbeExecutable::identity),
            "error": self.error,
        })
    }
}

#[derive(Debug, Clone)]
struct ResolvedProbeExecutable {
    path: PathBuf,
    path_digest: String,
    content_digest: String,
}

impl ResolvedProbeExecutable {
    fn identity(&self) -> Value {
        json!({
            "path": self.path.to_string_lossy(),
            "path_digest": self.path_digest,
            "content_digest": self.content_digest,
        })
    }
}

fn resolve_probe_executable(
    executable: &str,
    path: Option<&str>,
) -> Result<ResolvedProbeExecutable> {
    let path = path.context("PATH is required to resolve process executable")?;
    for directory in env::split_paths(path) {
        for candidate in probe_executable_candidates(&directory, executable) {
            if !candidate.is_file() {
                continue;
            }
            let canonical = candidate
                .canonicalize()
                .with_context(|| format!("canonicalizing executable {}", candidate.display()))?;
            let content = fs::read(&canonical)
                .with_context(|| format!("reading executable {}", canonical.display()))?;
            return Ok(ResolvedProbeExecutable {
                path_digest: sha256_prefixed_bytes(canonical.to_string_lossy().as_bytes()),
                content_digest: sha256_prefixed_bytes(&content),
                path: canonical,
            });
        }
    }
    bail!("process executable {executable} was not found in captured PATH");
}

fn probe_executable_candidates(directory: &Path, executable: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        windows_probe_executable_candidates(directory, executable, env::var("PATHEXT").ok())
    }
    #[cfg(not(windows))]
    {
        vec![directory.join(executable)]
    }
}

#[cfg(any(test, windows))]
fn windows_probe_executable_candidates(
    directory: &Path,
    executable: &str,
    pathext: Option<String>,
) -> Vec<PathBuf> {
    if Path::new(executable).extension().is_some() {
        return vec![directory.join(executable)];
    }
    windows_executable_suffixes(pathext)
        .into_iter()
        .map(|suffix| directory.join(format!("{executable}{suffix}")))
        .collect()
}

#[cfg(any(test, windows))]
fn windows_executable_suffixes(pathext: Option<String>) -> Vec<String> {
    let configured = pathext.unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut suffixes = Vec::new();
    for suffix in std::iter::once(".EXE".to_string()).chain(
        configured
            .split(';')
            .map(str::trim)
            .filter(|suffix| !suffix.is_empty())
            .map(|suffix| {
                if suffix.starts_with('.') {
                    suffix.to_string()
                } else {
                    format!(".{suffix}")
                }
            }),
    ) {
        if !suffixes
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&suffix))
        {
            suffixes.push(suffix);
        }
    }
    suffixes
}

fn run_process_probe(
    repository_root: &Path,
    execution: &ProcessExecutionContract,
    command_resolution: &ProbeCommandResolution,
    execution_id: String,
    observed_at: &str,
) -> ProbeOutcome {
    let cwd = match if let Some(working_directory) = execution.working_directory.as_deref() {
        contained_repository_path(repository_root, working_directory)
    } else {
        repository_root.canonicalize().map_err(anyhow::Error::from)
    } {
        Ok(path) => path,
        Err(error) => {
            return failed_probe(
                execution_id,
                observed_at.to_string(),
                error.to_string(),
                AttemptStatus::Failed,
                CapabilityAvailabilityStatus::Unavailable,
            );
        }
    };
    let resolved_executable = match &command_resolution.resolved {
        Some(resolved) => resolved,
        None => {
            return failed_probe(
                execution_id,
                observed_at.to_string(),
                command_resolution.error.clone().unwrap_or_else(|| {
                    "process executable could not be resolved from captured PATH".to_string()
                }),
                AttemptStatus::Unavailable,
                CapabilityAvailabilityStatus::Unavailable,
            );
        }
    };
    let mut argv = Vec::with_capacity(execution.args.len() + 1);
    argv.push(resolved_executable.path.to_string_lossy().to_string());
    argv.extend(execution.args.clone());
    let mut env = Vec::new();
    if let Some(path) = &command_resolution.path_value {
        env.push(("PATH", path.clone()));
    }
    let output = match run_bounded_process(BoundedProcessInput {
        cwd: &cwd,
        argv: &argv,
        env,
        timeout: Duration::from_millis(execution.timeout_ms),
        output_limit_bytes: usize::MAX,
        stdout_limit_bytes: Some(
            execution
                .stdout_limit_bytes
                .try_into()
                .unwrap_or(usize::MAX),
        ),
        stderr_limit_bytes: Some(
            execution
                .stderr_limit_bytes
                .try_into()
                .unwrap_or(usize::MAX),
        ),
        cancellation: &CancellationToken::new(),
    }) {
        Ok(output) => output,
        Err(error) => {
            let message = if error.to_string() == "output_limit_exceeded" {
                "probe output exceeded declared bounds".to_string()
            } else if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
                return failed_probe(
                    execution_id,
                    observed_at.to_string(),
                    format!("probe command unavailable: {io_error}"),
                    AttemptStatus::Unavailable,
                    availability_for_io_error(io_error),
                );
            } else {
                format!("probe command failed: {error}")
            };
            return failed_probe(
                execution_id,
                observed_at.to_string(),
                message,
                AttemptStatus::Failed,
                CapabilityAvailabilityStatus::ProbeFailed,
            );
        }
    };
    let outcome = if output.timed_out {
        AttemptStatus::TimedOut
    } else if output.interrupted {
        AttemptStatus::Aborted
    } else if output.exit_code == Some(0) {
        AttemptStatus::Passed
    } else {
        AttemptStatus::Failed
    };
    let boundary_signal = if outcome == AttemptStatus::Failed {
        adapter_boundary_signal_from_process_output(
            Some(output.stdout_excerpt.as_str()),
            Some(output.stderr_excerpt.as_str()),
        )
    } else {
        None
    };
    let availability = if outcome == AttemptStatus::Passed {
        CapabilityAvailability {
            status: CapabilityAvailabilityStatus::Available,
            reason: None,
        }
    } else if output.timed_out {
        CapabilityAvailability {
            status: CapabilityAvailabilityStatus::Unavailable,
            reason: Some("probe timed out".to_string()),
        }
    } else if output.exit_code == Some(126)
        || boundary_signal == Some(AdapterBoundarySignal::PermissionDenied)
    {
        CapabilityAvailability {
            status: CapabilityAvailabilityStatus::PermissionDenied,
            reason: Some("probe command reported permission denied".to_string()),
        }
    } else if boundary_signal == Some(AdapterBoundarySignal::SandboxBlocked) {
        CapabilityAvailability {
            status: CapabilityAvailabilityStatus::SandboxBlocked,
            reason: Some("probe command reported sandbox blocked".to_string()),
        }
    } else if output.exit_code == Some(127) {
        CapabilityAvailability {
            status: CapabilityAvailabilityStatus::Unavailable,
            reason: Some("probe command reported unavailable executable".to_string()),
        }
    } else {
        CapabilityAvailability {
            status: CapabilityAvailabilityStatus::ProbeFailed,
            reason: Some(format!("probe exited with status {:?}", output.exit_code)),
        }
    };
    ProbeOutcome {
        availability,
        result: ProbeResult {
            probe_execution_id: EvidenceId::parse(execution_id)
                .expect("probe execution id is valid"),
            outcome,
            observed_at: observed_at.to_string(),
            checks: vec![ProbeCheck {
                name: "process_probe".to_string(),
                outcome,
                detail: Some(format!(
                    "argv={:?}; stdout_digest={}; stderr_digest={}",
                    output.argv, output.stdout_digest, output.stderr_digest
                )),
            }],
        },
    }
}

fn failed_probe(
    execution_id: String,
    observed_at: String,
    reason: String,
    outcome: AttemptStatus,
    availability_status: CapabilityAvailabilityStatus,
) -> ProbeOutcome {
    ProbeOutcome {
        availability: CapabilityAvailability {
            status: availability_status,
            reason: Some(reason.clone()),
        },
        result: ProbeResult {
            probe_execution_id: EvidenceId::parse(execution_id)
                .expect("probe execution id is valid"),
            outcome,
            observed_at,
            checks: vec![ProbeCheck {
                name: "process_probe".to_string(),
                outcome,
                detail: Some(reason),
            }],
        },
    }
}

fn availability_for_io_error(error: &std::io::Error) -> CapabilityAvailabilityStatus {
    match error.kind() {
        ErrorKind::PermissionDenied => CapabilityAvailabilityStatus::PermissionDenied,
        ErrorKind::NotFound => CapabilityAvailabilityStatus::Unavailable,
        _ => CapabilityAvailabilityStatus::ProbeFailed,
    }
}

fn diagnostic_code_for_availability(
    status: CapabilityAvailabilityStatus,
) -> CapabilityRegistryDiagnosticCode {
    match status {
        CapabilityAvailabilityStatus::PermissionDenied => {
            CapabilityRegistryDiagnosticCode::PermissionDenied
        }
        _ => CapabilityRegistryDiagnosticCode::ProbeUnavailable,
    }
}

fn probe_execution_id(
    capability: &RegisteredCapability,
    runtime: &ValidatedCapabilityRuntimeContext,
    repository_root_digest: &str,
    command_resolution: &ProbeCommandResolution,
    observed_at: &str,
) -> Result<String> {
    let preimage = json!({
        "manifest_id": capability.manifest.id.as_str(),
        "manifest_digest": capability.manifest_digest,
        "adapter_version": capability.manifest.version,
        "execution_contract": capability.manifest.availability_probe.execution,
        "runtime": {
            "host": runtime.host,
            "surface": runtime.surface,
            "host_version": runtime.host_version,
            "environment_id": runtime.environment_id.as_str(),
            "repository_root": repository_root_digest,
            "command_resolution": command_resolution.identity(),
        },
        "observed_at": observed_at,
        "invocation_nonce": Uuid::new_v4().to_string(),
    });
    Ok(format!(
        "probe-{}",
        short_digest(sha256_json_digest(&preimage)?.as_bytes())
    ))
}

fn capability_instance(
    capability: &RegisteredCapability,
    runtime: &ValidatedCapabilityRuntimeContext,
    repository_root_digest: &str,
    command_resolution: &ProbeCommandResolution,
    probe: ProbeOutcome,
    captured_at: String,
) -> Result<VerificationCapabilityInstance> {
    let manifest = &capability.manifest;
    let payload_schema = &manifest.availability_probe.execution.payload_schema;
    let schema_ref = payload_schema.schema_ref.clone();
    let mut observation_types = Vec::with_capacity(manifest.supported_observations.len());
    for supported in &manifest.supported_observations {
        if supported.schema_ref != schema_ref {
            bail!("manifest supported_observations must share the availability probe schema_ref");
        }
        observation_types.push(supported.observation_type.clone());
    }
    let permissions = manifest_permission_state(manifest)?;
    let environment = capability_environment(
        capability,
        runtime,
        repository_root_digest,
        command_resolution,
    )?;
    let instance_id_suffix = short_digest(probe.result.probe_execution_id.as_str().as_bytes());
    let instance = VerificationCapabilityInstance {
        id: EvidenceId::parse(format!(
            "capinst-{}-{}",
            manifest.id.as_str().replace(['.', ':'], "-"),
            instance_id_suffix
        ))?,
        schema_version: SchemaVersion::v1(),
        manifest_id: manifest.id.clone(),
        manifest_digest: Sha256Digest::parse(capability.manifest_digest.clone())?,
        host: runtime.host.clone(),
        surface: runtime.surface.clone(),
        host_version: runtime.host_version.clone(),
        adapter_version: manifest.version.clone(),
        environment,
        permissions,
        availability: probe.availability,
        probe_result: probe.result,
        observed_payload_contract: ObservedPayloadContract {
            schema_ref,
            observation_types,
        },
        limitations: manifest.blind_spots.clone(),
        captured_at,
    };
    let value = serde_json::to_value(&instance)?;
    serde_json::from_value(value).context("constructed capability instance must satisfy contract")
}

fn capability_environment(
    capability: &RegisteredCapability,
    runtime: &ValidatedCapabilityRuntimeContext,
    repository_root_digest: &str,
    command_resolution: &ProbeCommandResolution,
) -> Result<EnvironmentBinding> {
    let environment_value = json!({
        "host": runtime.host,
        "surface": runtime.surface,
        "host_version": runtime.host_version,
        "adapter_version": capability.manifest.version,
        "repository_root": repository_root_digest,
        "command_resolution": command_resolution.identity(),
    });
    Ok(EnvironmentBinding {
        kind: "local".to_string(),
        id: runtime.environment_id.clone(),
        digest: Sha256Digest::parse(sha256_json_digest(&environment_value)?)?,
    })
}

fn current_compatible_capability_instance(
    conn: &Connection,
    capability: &RegisteredCapability,
    runtime: &ValidatedCapabilityRuntimeContext,
    repository_root_digest: &str,
    command_resolution: &ProbeCommandResolution,
) -> Result<CurrentCapabilityLookup> {
    let expected_runtime_targets = serde_json::to_value(&capability.manifest.runtime_targets)?;
    let expected_environment = capability_environment(
        capability,
        runtime,
        repository_root_digest,
        command_resolution,
    )?;
    let expected_execution_contract = capability
        .repository_execution_contract
        .as_ref()
        .unwrap_or(&capability.manifest.availability_probe.execution);
    let expected_execution_contract_digest =
        sha256_json_digest(&serde_json::to_value(expected_execution_contract)?)?;
    let expected_execution_contract_source = if capability.repository_execution_contract.is_some() {
        "repository_adapter_registration"
    } else {
        "manifest_availability_probe"
    };
    let mut statement = conn.prepare(
        "SELECT runtime_target_json, host_fingerprint_json, capability_snapshot_json, valid_until
         FROM verification_capability_instances
         WHERE manifest_id = ?1
           AND manifest_version = ?2
           AND manifest_digest = ?3
           AND availability_status = 'available'
         ORDER BY created_at DESC, id DESC",
    )?;
    let rows = statement
        .query_map(
            params![
                capability.manifest.id.as_str(),
                capability.manifest.version.as_str(),
                capability.manifest_digest.as_str()
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let now = OffsetDateTime::now_utc();
    let mut miss_reason = CapabilityInstanceResolutionReason::ProbedNoCurrent;
    for (runtime_target_json, host_fingerprint_json, capability_snapshot_json, valid_until) in rows
    {
        if let Some(valid_until) = valid_until {
            match OffsetDateTime::parse(&valid_until, &Rfc3339) {
                Ok(expires) if expires > now => {}
                _ => {
                    miss_reason = CapabilityInstanceResolutionReason::ReprobedExpired;
                    continue;
                }
            }
        }
        let runtime_target: Value = serde_json::from_str(&runtime_target_json)?;
        if runtime_target != expected_runtime_targets {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedRuntimeMismatch;
            continue;
        }
        let host_fingerprint: Value = serde_json::from_str(&host_fingerprint_json)?;
        if host_fingerprint
            .get("execution_contract_digest")
            .and_then(Value::as_str)
            != Some(expected_execution_contract_digest.as_str())
            || host_fingerprint
                .get("execution_contract_source")
                .and_then(Value::as_str)
                != Some(expected_execution_contract_source)
        {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedRegistrationMismatch;
            continue;
        }
        let instance: VerificationCapabilityInstance =
            serde_json::from_str(&capability_snapshot_json)?;
        if instance.manifest_id.as_str() != capability.manifest.id.as_str()
            || instance.manifest_digest.as_str() != capability.manifest_digest
            || instance.adapter_version != capability.manifest.version
        {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedRegistrationMismatch;
            continue;
        }
        if instance.host != runtime.host
            || instance.surface != runtime.surface
            || instance.host_version != runtime.host_version
        {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedRuntimeMismatch;
            continue;
        }
        if instance.environment != expected_environment
            || serde_json::to_value(&instance.environment)? != host_fingerprint["environment"]
        {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedEnvironmentMismatch;
            continue;
        }
        if instance.availability.status != CapabilityAvailabilityStatus::Available {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedRegistrationMismatch;
            continue;
        }
        let payload_schema = &capability
            .manifest
            .availability_probe
            .execution
            .payload_schema;
        if instance.observed_payload_contract.schema_ref != payload_schema.schema_ref
            || !instance
                .observed_payload_contract
                .observation_types
                .iter()
                .any(|observed| observed.as_str() == payload_schema.observation_type.as_str())
        {
            miss_reason = CapabilityInstanceResolutionReason::ReprobedRegistrationMismatch;
            continue;
        }
        return Ok(CurrentCapabilityLookup {
            instance: Some(instance),
            miss_reason: CapabilityInstanceResolutionReason::ReusedCurrent,
        });
    }
    Ok(CurrentCapabilityLookup {
        instance: None,
        miss_reason,
    })
}

struct CurrentCapabilityLookup {
    instance: Option<VerificationCapabilityInstance>,
    miss_reason: CapabilityInstanceResolutionReason,
}

fn store_capability_manifest(conn: &Connection, capability: &RegisteredCapability) -> Result<()> {
    let manifest_json = serde_json::to_string(&capability.manifest)?;
    let existing = conn
        .query_row(
            "SELECT manifest_digest, manifest_json
             FROM verification_capability_manifests
             WHERE id = ?1 AND version = ?2",
            params![
                capability.manifest.id.as_str(),
                capability.manifest.version.as_str()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((existing_digest, existing_json)) = existing {
        if existing_digest != capability.manifest_digest || existing_json != manifest_json {
            bail!(
                "verification capability manifest {}@{} digest/content mismatch: existing {}, incoming {}",
                capability.manifest.id.as_str(),
                capability.manifest.version.as_str(),
                existing_digest,
                capability.manifest_digest
            );
        }
        return Ok(());
    }

    conn.execute(
        "INSERT INTO verification_capability_manifests(
          id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, source_path, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            capability.manifest.id.as_str(),
            capability.manifest.version.as_str(),
            capability.manifest.adapter_kind.as_str(),
            capability.manifest.adapter_digest.as_str(),
            capability.manifest_digest.as_str(),
            manifest_json,
            match &capability.source {
                CapabilitySource::BuiltIn => None,
                CapabilitySource::Repository { manifest_path } => Some(manifest_path.as_str()),
            },
            timestamp(),
        ],
    )?;
    Ok(())
}

fn store_capability_instance(
    conn: &Connection,
    instance: &VerificationCapabilityInstance,
    capability: &RegisteredCapability,
) -> Result<()> {
    store_capability_instance_with_expiry(conn, instance, capability, None)
}

fn store_capability_instance_with_expiry(
    conn: &Connection,
    instance: &VerificationCapabilityInstance,
    capability: &RegisteredCapability,
    valid_until: Option<&str>,
) -> Result<()> {
    let canonical_execution_contract = capability
        .repository_execution_contract
        .as_ref()
        .unwrap_or(&capability.manifest.availability_probe.execution);
    let host_fingerprint = json!({
        "environment": instance.environment,
        "execution_contract_digest": sha256_json_digest(&serde_json::to_value(canonical_execution_contract)?)?,
        "execution_contract_source": if capability.repository_execution_contract.is_some() {
            "repository_adapter_registration"
        } else {
            "manifest_availability_probe"
        }
    });
    let instance_json = serde_json::to_string(instance)?;
    let probe_json = serde_json::to_string(&instance.probe_result)?;
    let existing = conn
        .query_row(
            "SELECT manifest_id, manifest_version, manifest_digest, capability_snapshot_json, probe_result_json
             FROM verification_capability_instances WHERE id = ?1",
            params![instance.id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    if let Some((
        manifest_id,
        manifest_version,
        manifest_digest,
        existing_instance,
        existing_probe,
    )) = existing
    {
        if manifest_id == instance.manifest_id.as_str()
            && manifest_version == capability.manifest.version
            && manifest_digest == instance.manifest_digest.as_str()
            && existing_instance == instance_json
            && existing_probe == probe_json
        {
            return Ok(());
        }
        bail!(
            "verification capability instance {} digest/content mismatch",
            instance.id.as_str()
        );
    }
    conn.execute(
        "INSERT INTO verification_capability_instances(
          id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
          availability_status, runtime_target_json, host_fingerprint_json, capability_snapshot_json,
          probe_result_json, created_at, valid_until
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            instance.id.as_str(),
            instance.manifest_id.as_str(),
            capability.manifest.version.as_str(),
            instance.manifest_digest.as_str(),
            instance.probe_result.probe_execution_id.as_str(),
            instance.availability.status.as_str(),
            serde_json::to_string(&capability.manifest.runtime_targets)?,
            serde_json::to_string(&host_fingerprint)?,
            instance_json,
            probe_json,
            instance.captured_at,
            valid_until,
        ],
    )?;
    Ok(())
}

fn timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("current UTC time formats as RFC3339")
}

fn short_digest(bytes: &[u8]) -> String {
    sha256_prefixed_bytes(bytes)
        .trim_start_matches("sha256:")
        .chars()
        .take(12)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;
    use std::{
        process::{Command as StdCommand, Stdio as StdStdio},
        thread,
        time::Instant,
    };
    use tempfile::tempdir;

    const DIGEST: &str = "sha256:6666666666666666666666666666666666666666666666666666666666666666";
    const SCHEMA_DIGEST: &str =
        "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const OTHER_SCHEMA_DIGEST: &str =
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    fn manifest_value(executable: &str, args: Vec<&str>, stdout_limit_bytes: u64) -> Value {
        manifest_value_with_id(
            "vcap-test-process-v1",
            executable,
            args,
            stdout_limit_bytes,
            1024,
            5000,
        )
    }

    fn manifest_value_with_id(
        id: &str,
        executable: &str,
        args: Vec<&str>,
        stdout_limit_bytes: u64,
        stderr_limit_bytes: u64,
        timeout_ms: u64,
    ) -> Value {
        json!({
            "id": id,
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "process",
            "adapter_digest": DIGEST,
            "supported_surfaces": ["local-process"],
            "supported_observations": [{
                "type": "planr.test.output",
                "schema_ref": "planr.test.output@v1",
                "schema_digest": SCHEMA_DIGEST
            }],
            "supported_interactions": ["process"],
            "supported_artifacts": ["stdout"],
            "runtime_targets": [{"kind": "process", "id": "test"}],
            "provenance_path": "planr_observed_execution",
            "permissions": {"network": "none", "filesystem": "read_workspace"},
            "costs": {},
            "determinism": "deterministic",
            "repeatability": "repeatable",
            "independence": "independent",
            "blind_spots": ["none"],
            "availability_probe": {
                "kind": "process",
                "execution": {
                    "kind": "process",
                    "executable": executable,
                    "args": args,
                    "working_directory": ".",
                    "timeout_ms": timeout_ms,
                    "stdout_limit_bytes": stdout_limit_bytes,
                    "stderr_limit_bytes": stderr_limit_bytes,
                    "payload_schema": {
                        "type": "planr.test.output",
                        "schema_ref": "planr.test.output@v1",
                        "schema_digest": SCHEMA_DIGEST
                    }
                }
            }
        })
    }

    fn registration_for(manifest_digest: &str) -> Value {
        registration_for_manifest(
            manifest_digest,
            &manifest_value("cargo", vec!["--version"], 1024),
        )
    }

    fn registration_for_manifest(manifest_digest: &str, manifest: &Value) -> Value {
        let supported = manifest["supported_observations"].as_array().unwrap();
        json!({
            "manifest_id": manifest["id"],
            "manifest_path": "adapters/test-manifest.json",
            "manifest_digest": manifest_digest,
            "observation_types": supported
                .iter()
                .map(|binding| binding["type"].clone())
                .collect::<Vec<_>>(),
            "payload_schemas": manifest["supported_observations"],
            "provenance_path": manifest["provenance_path"],
            "execution_contract": repository_execution_contract_for(manifest)
        })
    }

    fn repository_execution_contract_for(manifest: &Value) -> Value {
        json!({
            "kind": "process",
            "executable": "cargo",
            "args": ["--version", "--verbose"],
            "working_directory": ".",
            "timeout_ms": 10000,
            "stdout_limit_bytes": 2048,
            "stderr_limit_bytes": 2048,
            "payload_schema": manifest["supported_observations"][0]
        })
    }

    fn write_manifest(root: &Path, value: &Value) -> String {
        let path = root.join("adapters/test-manifest.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
        sha256_json_digest(value).unwrap()
    }

    fn manifest(value: Value) -> VerificationCapabilityManifest {
        serde_json::from_value(value).unwrap()
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE verification_capability_manifests(
              id TEXT NOT NULL,
              version TEXT NOT NULL,
              adapter_kind TEXT NOT NULL,
              adapter_digest TEXT NOT NULL,
              manifest_digest TEXT NOT NULL UNIQUE,
              manifest_json TEXT NOT NULL,
              source_path TEXT,
              created_at TEXT NOT NULL,
              PRIMARY KEY(id, version)
            );
            CREATE TABLE verification_capability_instances(
              id TEXT PRIMARY KEY,
              manifest_id TEXT NOT NULL,
              manifest_version TEXT NOT NULL,
              manifest_digest TEXT NOT NULL,
              probe_execution_id TEXT NOT NULL,
              availability_status TEXT NOT NULL,
              runtime_target_json TEXT NOT NULL,
              host_fingerprint_json TEXT NOT NULL DEFAULT '{}',
              capability_snapshot_json TEXT NOT NULL,
              probe_result_json TEXT NOT NULL,
              created_at TEXT NOT NULL,
              valid_until TEXT,
              FOREIGN KEY(manifest_id, manifest_version) REFERENCES verification_capability_manifests(id, version),
              FOREIGN KEY(manifest_digest) REFERENCES verification_capability_manifests(manifest_digest),
              UNIQUE(manifest_id, manifest_version, probe_execution_id)
            );
            "#,
        )
        .unwrap();
        conn
    }

    fn assert_invalid_root_registry_cannot_persist_or_run(
        registry: &mut CapabilityRegistry,
        conn: &Connection,
        probe_root: &Path,
        marker: &Path,
    ) {
        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::InvalidRepositoryRoot
        );

        let error = registry
            .probe_and_store(conn, probe_root, "vcap-test-process-v1", runtime())
            .unwrap_err()
            .to_string();

        assert!(error.contains("must canonicalize to an existing directory"));
        assert!(!marker.exists(), "invalid root launched availability probe");
        assert!(
            registry.runtime_availability.is_empty(),
            "invalid root mutated availability cache"
        );
        let manifest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_manifests",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let instance_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(manifest_count, 0, "invalid root persisted manifest row");
        assert_eq!(instance_count, 0, "invalid root persisted instance row");
    }

    fn diagnostic_count(
        registry: &CapabilityRegistry,
        code: CapabilityRegistryDiagnosticCode,
    ) -> usize {
        registry
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code == code)
            .count()
    }

    #[test]
    fn registry_rejects_missing_relative_repository_root_before_registration() {
        let root_name = format!("target/missing-registry-root-{}", Uuid::new_v4());
        let relative_root = std::path::PathBuf::from(root_name);
        assert!(!relative_root.exists());
        let marker = std::env::current_dir().unwrap().join(format!(
            "target/missing-relative-root-{}.marker",
            Uuid::new_v4()
        ));
        let script = format!("touch {}", marker.display());

        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            &relative_root,
            [manifest(manifest_value(
                "sh",
                vec!["-c", script.as_str()],
                1024,
            ))],
            &[],
        );
        fs::create_dir_all(&relative_root).unwrap();
        let conn = conn();

        assert_invalid_root_registry_cannot_persist_or_run(
            &mut registry,
            &conn,
            &relative_root,
            &marker,
        );

        fs::remove_dir_all(&relative_root).unwrap();
    }

    #[test]
    fn registry_rejects_missing_absolute_repository_root_before_registration() {
        let dir = tempdir().unwrap();
        let absolute_root = dir.path().join("future-repo");
        let marker = dir.path().join("missing-absolute-root-ran.marker");
        let script = format!("touch {}", marker.display());

        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            &absolute_root,
            [manifest(manifest_value(
                "sh",
                vec!["-c", script.as_str()],
                1024,
            ))],
            &[],
        );
        fs::create_dir_all(&absolute_root).unwrap();
        let conn = conn();

        assert_invalid_root_registry_cannot_persist_or_run(
            &mut registry,
            &conn,
            &absolute_root,
            &marker,
        );
    }

    #[test]
    fn registry_rejects_file_repository_root_before_registration() {
        let dir = tempdir().unwrap();
        let file_root = dir.path().join("not-a-directory");
        fs::write(&file_root, b"not a repository root").unwrap();
        let marker = dir.path().join("file-root-ran.marker");
        let script = format!("touch {}", marker.display());

        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            &file_root,
            [manifest(manifest_value(
                "sh",
                vec!["-c", script.as_str()],
                1024,
            ))],
            &[],
        );
        let conn = conn();

        assert_invalid_root_registry_cannot_persist_or_run(
            &mut registry,
            &conn,
            &file_root,
            &marker,
        );
    }

    #[test]
    fn registry_merges_built_in_and_repository_capabilities() {
        let dir = tempdir().unwrap();
        let base_manifest = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &base_manifest);
        let mut built_in_value = manifest_value_with_id(
            "vcap-built-in-v1",
            "cargo",
            vec!["--version"],
            1024,
            1024,
            5000,
        );
        built_in_value["supported_observations"][0]["type"] = json!("planr.test.builtin");
        built_in_value["supported_observations"][0]["schema_ref"] = json!("planr.test.builtin@v1");
        built_in_value["availability_probe"]["execution"]["payload_schema"]["type"] =
            json!("planr.test.builtin");
        built_in_value["availability_probe"]["execution"]["payload_schema"]["schema_ref"] =
            json!("planr.test.builtin@v1");
        let built_in = manifest(built_in_value);

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [built_in],
            &[registration_for(&digest)],
        );

        assert!(
            registry.diagnostics().is_empty(),
            "{:?}",
            registry.diagnostics()
        );
        let ids = registry
            .capabilities()
            .map(|capability| capability.manifest.id.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["vcap-built-in-v1", "vcap-test-process-v1"]);
    }

    #[test]
    fn registry_rejects_duplicate_builtin_manifest_ids() {
        let first = manifest(manifest_value("cargo", vec!["--version"], 1024));
        let second = manifest(manifest_value("cargo", vec!["--version"], 1024));

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            Path::new("."),
            [first, second],
            &[],
        );

        assert_eq!(registry.capabilities().count(), 1);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DuplicateManifestId
        );
    }

    #[test]
    fn registry_builtin_manifest_digest_changes_when_manifest_fields_change() {
        let first = manifest(manifest_value("cargo", vec!["--version"], 1024));
        let mut changed_value = manifest_value("cargo", vec!["--version"], 1024);
        changed_value["blind_spots"] = json!(["changed built-in blind spot"]);
        let changed = manifest(changed_value);

        let first_registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            Path::new("."),
            [first],
            &[],
        );
        let changed_registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            Path::new("."),
            [changed],
            &[],
        );

        assert!(
            first_registry.diagnostics().is_empty(),
            "{:?}",
            first_registry.diagnostics()
        );
        assert!(
            changed_registry.diagnostics().is_empty(),
            "{:?}",
            changed_registry.diagnostics()
        );
        let first_digest = first_registry
            .capabilities()
            .next()
            .unwrap()
            .manifest_digest
            .as_str();
        let changed_digest = changed_registry
            .capabilities()
            .next()
            .unwrap()
            .manifest_digest
            .as_str();
        assert_ne!(first_digest, DIGEST);
        assert_ne!(changed_digest, DIGEST);
        assert_ne!(first_digest, changed_digest);
    }

    #[test]
    fn registry_rejects_duplicate_repository_manifest_ids() {
        let dir = tempdir().unwrap();
        let base_manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &base_manifest_value);
        let registration = registration_for_manifest(&digest, &base_manifest_value);

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration.clone(), registration],
        );

        assert_eq!(registry.capabilities().count(), 1);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DuplicateManifestId
        );
    }

    #[test]
    fn registry_rejects_repository_manifest_that_shadows_builtin() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let built_in = manifest(manifest_value.clone());

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [built_in],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        assert_eq!(registry.capabilities().count(), 1);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DuplicateManifestId
        );
        assert!(matches!(
            registry.capabilities().next().unwrap().source,
            CapabilitySource::BuiltIn
        ));
    }

    #[test]
    fn registry_reports_declared_manifest_digest_drift() {
        let dir = tempdir().unwrap();
        write_manifest(
            dir.path(),
            &manifest_value("cargo", vec!["--version"], 1024),
        );

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(registry.diagnostics()[0].message.contains("actual sha256:"));
    }

    #[test]
    fn registry_retains_repository_execution_contract_but_probes_manifest_availability() {
        let dir = tempdir().unwrap();
        let probe_marker = dir.path().join("availability-probe-ran.marker");
        let repository_marker = dir.path().join("repository-execution-ran.marker");
        let probe_script = format!("touch {}", probe_marker.display());
        let repository_script = format!("touch {}", repository_marker.display());
        let manifest_value = manifest_value("sh", vec!["-c", probe_script.as_str()], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registration = registration_for_manifest(&digest, &manifest_value);
        registration["execution_contract"] = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", repository_script],
            "working_directory": ".",
            "timeout_ms": 30000,
            "stdout_limit_bytes": 8192,
            "stderr_limit_bytes": 8192,
            "payload_schema": manifest_value["supported_observations"][0]
        });
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration],
        );

        assert!(
            registry.diagnostics().is_empty(),
            "{:?}",
            registry.diagnostics()
        );
        let capability = registry.capabilities().next().unwrap();
        let repository_execution = capability.repository_execution_contract.as_ref().unwrap();
        assert_eq!(repository_execution.executable, "sh");
        assert_eq!(repository_execution.timeout_ms, 30000);
        assert_ne!(
            serde_json::to_value(repository_execution).unwrap(),
            serde_json::to_value(&capability.manifest.availability_probe.execution).unwrap()
        );

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Available
        );
        assert!(probe_marker.exists(), "availability probe did not run");
        assert!(
            !repository_marker.exists(),
            "repository execution contract was used for availability probing"
        );
    }

    #[test]
    fn registry_rejects_repository_execution_contract_payload_schema_drift() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registration = registration_for_manifest(&digest, &manifest_value);
        registration["execution_contract"]["payload_schema"]["type"] = json!("planr.test.other");

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(
            registry.diagnostics()[0]
                .message
                .contains("execution_contract payload_schema"),
            "{:?}",
            registry.diagnostics()
        );
    }

    #[test]
    fn registry_rejects_builtin_availability_payload_schema_drift() {
        let mut manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        manifest_value["availability_probe"]["execution"]["payload_schema"]["type"] =
            json!("planr.test.other");

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            Path::new("."),
            [manifest(manifest_value)],
            &[],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(
            registry.diagnostics()[0]
                .message
                .contains("availability_probe.execution payload_schema"),
            "{:?}",
            registry.diagnostics()
        );
    }

    #[test]
    fn registry_rejects_repository_availability_payload_schema_drift() {
        let dir = tempdir().unwrap();
        let mut manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        manifest_value["availability_probe"]["execution"]["payload_schema"]["schema_ref"] =
            json!("planr.test.other@v1");
        let digest = write_manifest(dir.path(), &manifest_value);

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(
            registry.diagnostics()[0]
                .message
                .contains("availability_probe.execution payload_schema"),
            "{:?}",
            registry.diagnostics()
        );
    }

    #[test]
    fn registry_instance_projects_exact_availability_payload_binding_for_multi_schema_manifest() {
        let dir = tempdir().unwrap();
        let mut manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        manifest_value["supported_observations"] = json!([
            {
                "type": "planr.test.alpha",
                "schema_ref": "planr.test.browser@v1",
                "schema_digest": SCHEMA_DIGEST
            },
            {
                "type": "planr.test.beta",
                "schema_ref": "planr.test.browser@v1",
                "schema_digest": SCHEMA_DIGEST
            }
        ]);
        manifest_value["availability_probe"]["execution"]["payload_schema"] =
            manifest_value["supported_observations"][1].clone();
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.observed_payload_contract.schema_ref,
            "planr.test.browser@v1"
        );
        assert_eq!(
            instance.observed_payload_contract.observation_types.len(),
            2
        );
        let observed = instance
            .observed_payload_contract
            .observation_types
            .iter()
            .map(|value| value.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            observed,
            BTreeSet::from(["planr.test.alpha", "planr.test.beta"])
        );
    }

    #[test]
    fn registry_rejects_unsafe_manifest_paths_and_executables() {
        let dir = tempdir().unwrap();
        let digest = write_manifest(
            dir.path(),
            &manifest_value("cargo", vec!["--version"], 1024),
        );
        let mut unsafe_path = registration_for(&digest);
        unsafe_path["manifest_path"] = json!("../outside.json");
        let mut unsafe_command = registration_for(&digest);
        unsafe_command["execution_contract"]["executable"] = json!("./adapter");

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[unsafe_path, unsafe_command],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::UnsafeManifestPath
        );
        assert_eq!(
            registry.diagnostics()[1].code,
            CapabilityRegistryDiagnosticCode::UnsafeExecutionContract
        );
    }

    #[cfg(unix)]
    #[test]
    fn registry_rejects_unsafe_builtin_executable_before_probe() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let marker = dir.path().join("unsafe-builtin-executable-ran.marker");
        let adapter = dir.path().join("adapter");
        fs::write(&adapter, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [manifest(manifest_value("./adapter", vec![], 1024))],
            &[],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::UnsafeExecutionContract
        );
        assert!(
            !marker.exists(),
            "unsafe built-in executable ran during registration"
        );
    }

    #[test]
    fn registry_rejects_unsafe_builtin_working_directory_before_probe() {
        let dir = tempdir().unwrap();
        let marker = dir
            .path()
            .join("unsafe-builtin-working-directory-ran.marker");
        let script = format!("touch {}", marker.display());
        let mut value = manifest_value("sh", vec!["-c", script.as_str()], 1024);
        value["availability_probe"]["execution"]["working_directory"] = json!("../outside");

        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [manifest(value)],
            &[],
        );

        assert_eq!(registry.capabilities().count(), 0);
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::UnsafeExecutionContract
        );
        assert!(
            !marker.exists(),
            "unsafe built-in working directory probe was launched"
        );
    }

    #[test]
    fn registry_probe_persists_runtime_capability_instance() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for(&digest)],
        );
        let conn = conn();

        let instance = registry
            .probe_and_store(
                &conn,
                dir.path(),
                "vcap-test-process-v1",
                CapabilityRuntimeContext {
                    host: "codex",
                    surface: "local-process",
                    host_version: "test",
                    environment_id: "env-test",
                },
            )
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Available
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances WHERE manifest_id = 'vcap-test-process-v1' AND availability_status = 'available'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn registry_current_resolution_reuses_only_compatible_instances() {
        let dir = tempdir().unwrap();
        let base_manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &base_manifest_value);
        let conn = conn();
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for(&digest)],
        );

        let first = registry
            .current_or_probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        assert!(!first.reused);
        let second = registry
            .current_or_probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        assert!(second.reused);
        assert_eq!(second.instance.id, first.instance.id);

        let changed_runtime = CapabilityRuntimeContext {
            environment_id: "env-other",
            ..runtime()
        };
        let third = registry
            .current_or_probe_and_store(&conn, dir.path(), "vcap-test-process-v1", changed_runtime)
            .unwrap();
        assert!(!third.reused);
        assert_ne!(third.instance.id, first.instance.id);

        let changed_manifest_value = manifest_value("cargo", vec!["--version"], 2048);
        let changed_digest = write_manifest(dir.path(), &changed_manifest_value);
        let mut changed_registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(
                &changed_digest,
                &changed_manifest_value,
            )],
        );
        assert!(
            changed_registry
                .current_or_probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
                .is_err(),
            "same manifest id/version with a changed digest must not reuse stale instances"
        );
    }

    #[test]
    fn registry_current_resolution_skips_expired_instances_and_recovers() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let conn = conn();
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for(&digest)],
        );

        let first = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        conn.execute(
            "UPDATE verification_capability_instances SET valid_until = '2000-01-01T00:00:00Z' WHERE id = ?1",
            [first.id.as_str()],
        )
        .unwrap();
        let recovered = registry
            .current_or_probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        assert!(!recovered.reused);
        assert_ne!(recovered.instance.id, first.id);
        let reused = registry
            .current_or_probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        assert!(reused.reused);
        assert_eq!(reused.instance.id, recovered.instance.id);
    }

    #[test]
    fn registry_unsupported_surface_persists_snapshot_without_running_probe() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("unsupported-surface-ran.marker");
        let script = format!("touch {}", marker.display());
        let manifest_value = manifest_value("sh", vec!["-c", script.as_str()], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let instance = registry
            .probe_and_store(
                &conn,
                dir.path(),
                "vcap-test-process-v1",
                CapabilityRuntimeContext {
                    surface: "cli",
                    ..runtime()
                },
            )
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Unsupported
        );
        assert_eq!(instance.probe_result.outcome, AttemptStatus::Skipped);
        assert!(
            !marker.exists(),
            "unsupported surface launched the availability probe"
        );
        assert_eq!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                [0]
            .code,
            CapabilityRegistryDiagnosticCode::ProbeUnavailable
        );
        let stored: String = conn
            .query_row(
                "SELECT availability_status FROM verification_capability_instances WHERE id = ?1",
                [instance.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "unsupported");
    }

    #[test]
    fn registry_invalid_runtime_contexts_do_not_persist_or_run_probe() {
        for (label, runtime, expected) in [
            (
                "empty-host",
                CapabilityRuntimeContext {
                    host: "",
                    ..runtime()
                },
                "runtime host",
            ),
            (
                "empty-surface",
                CapabilityRuntimeContext {
                    surface: "",
                    ..runtime()
                },
                "runtime surface",
            ),
            (
                "empty-host-version",
                CapabilityRuntimeContext {
                    host_version: "",
                    ..runtime()
                },
                "runtime host_version",
            ),
            (
                "malformed-environment",
                CapabilityRuntimeContext {
                    environment_id: "bad env id",
                    ..runtime()
                },
                "runtime environment_id",
            ),
        ] {
            let dir = tempdir().unwrap();
            let marker = dir.path().join(format!("{label}-ran.marker"));
            let script = format!("touch {}", marker.display());
            let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
                dir.path(),
                [manifest(manifest_value(
                    "sh",
                    vec!["-c", script.as_str()],
                    1024,
                ))],
                &[],
            );
            let conn = conn();

            let error = registry
                .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime)
                .unwrap_err()
                .to_string();

            assert!(error.contains(expected), "{label}: {error}");
            assert!(!marker.exists(), "{label}: invalid runtime launched probe");
            assert!(
                registry.runtime_availability.is_empty(),
                "{label}: invalid runtime mutated availability cache"
            );
            let manifest_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM verification_capability_manifests",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let instance_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM verification_capability_instances",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(manifest_count, 0, "{label}: manifest row persisted");
            assert_eq!(instance_count, 0, "{label}: instance row persisted");
        }
    }

    #[test]
    fn registry_rejects_probe_repository_root_mismatch_before_side_effects() {
        let repo_a = tempdir().unwrap();
        let repo_b = tempdir().unwrap();
        let marker = repo_b.path().join("wrong-root-probe-ran.marker");
        let script = format!("touch {}", marker.display());
        let manifest_value = manifest_value("sh", vec!["-c", script.as_str()], 1024);
        let digest = write_manifest(repo_a.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            repo_a.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let error = registry
            .probe_and_store(&conn, repo_b.path(), "vcap-test-process-v1", runtime())
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("does not match bound registry root"),
            "{error}"
        );
        assert!(!marker.exists(), "mismatched probe root launched process");
        assert!(
            registry.runtime_availability.is_empty(),
            "mismatched probe root mutated availability cache"
        );
        let manifest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_manifests",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let instance_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(manifest_count, 0);
        assert_eq!(instance_count, 0);
    }

    #[test]
    fn registry_rejects_persisted_same_version_manifest_drift() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("drifted-adapter-ran.marker");
        let mut first_registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [manifest(manifest_value("cargo", vec!["--version"], 1024))],
            &[],
        );
        let marker_script = format!("touch {}", marker.display());
        let mut changed_value = manifest_value("sh", vec!["-c", marker_script.as_str()], 1024);
        changed_value["blind_spots"] = json!(["same version changed built-in manifest content"]);
        let mut changed_registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [manifest(changed_value)],
            &[],
        );
        let conn = conn();

        first_registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        let error = changed_registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap_err()
            .to_string();

        assert!(error.contains("digest/content mismatch"), "{error}");
        assert!(
            !marker.exists(),
            "drifted manifest executable ran before manifest identity preflight"
        );
        let manifest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_manifests WHERE id = 'vcap-test-process-v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let instance_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances WHERE manifest_id = 'vcap-test-process-v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(manifest_count, 1);
        assert_eq!(instance_count, 1);
    }

    #[test]
    fn registry_probe_records_bounded_output_failure() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value("cargo", vec!["--version"], 1);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registration = registration_for_manifest(&digest, &manifest_value);
        registration["execution_contract"]["stdout_limit_bytes"] = json!(1);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration],
        );
        let conn = conn();

        let instance = registry
            .probe_and_store(
                &conn,
                dir.path(),
                "vcap-test-process-v1",
                CapabilityRuntimeContext {
                    host: "codex",
                    surface: "local-process",
                    host_version: "test",
                    environment_id: "env-test",
                },
            )
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::ProbeFailed
        );
        assert!(registry.diagnostics().iter().any(
            |diagnostic| diagnostic.code == CapabilityRegistryDiagnosticCode::ProbeUnavailable
        ));
        let availability = registry
            .available_diagnostics_for_declared_observations(["planr.test.output".to_string()]);
        assert_eq!(availability.len(), 1);
        assert_eq!(
            availability[0].code,
            CapabilityRegistryDiagnosticCode::ProbeUnavailable
        );
    }

    #[test]
    fn registry_declared_availability_uses_runtime_probe_status() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        let not_probed = registry
            .available_diagnostics_for_declared_observations(["planr.test.output".to_string()]);
        assert_eq!(
            not_probed[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestUnavailable
        );
    }

    #[test]
    fn registry_probe_missing_executable_persists_unavailable_and_diagnostic() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value(
            "planr-command-that-should-not-exist",
            vec!["--version"],
            1024,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let instance = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Unavailable
        );
        assert_eq!(instance.probe_result.outcome, AttemptStatus::Unavailable);
        assert_eq!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                [0]
            .code,
            CapabilityRegistryDiagnosticCode::ProbeUnavailable
        );
        let stored: String = conn
            .query_row(
                "SELECT availability_status FROM verification_capability_instances WHERE id = ?1",
                [instance.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "unavailable");
    }

    #[cfg(unix)]
    #[test]
    fn registry_probe_permission_denied_persists_and_diagnoses_permission() {
        let dir = tempdir().unwrap();
        let denied = dir.path().join("denied-adapter");
        fs::write(&denied, "#!/bin/sh\necho denied\n").unwrap();
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", "./denied-adapter"],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let registration = registration_for_manifest(&digest, &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration],
        );
        let conn = conn();

        let instance = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::PermissionDenied
        );
        assert_eq!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                [0]
            .code,
            CapabilityRegistryDiagnosticCode::PermissionDenied
        );
        let stored: String = conn
            .query_row(
                "SELECT availability_status FROM verification_capability_instances WHERE id = ?1",
                [instance.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "permission_denied");
    }

    #[test]
    fn registry_probe_maps_sandbox_boundary_from_structured_signal() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec![
                "-c",
                "printf '{\"planr_adapter_boundary\":\"sandbox_blocked\"}\\n' >&2; exit 2",
            ],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let registration = registration_for_manifest(&digest, &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration],
        );
        let conn = conn();

        let instance = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::SandboxBlocked
        );
        let stored: String = conn
            .query_row(
                "SELECT availability_status FROM verification_capability_instances WHERE id = ?1",
                [instance.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "sandbox_blocked");
    }

    #[test]
    fn registry_missing_permissions_do_not_register_or_spawn_adapter() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("adapter-ran.marker");
        let script = format!("touch {}", marker.display());
        let mut manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", &script],
            1024,
            1024,
            5000,
        );
        manifest_value["permissions"]
            .as_object_mut()
            .unwrap()
            .remove("network");
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let error = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap_err()
            .to_string();

        assert!(error.contains("is not registered"), "{error}");
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(
            registry.diagnostics()[0]
                .message
                .contains("permissions.network"),
            "{:?}",
            registry.diagnostics()
        );
        assert!(!marker.exists(), "invalid-permission adapter was executed");
        assert_eq!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                [0]
            .code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestUnavailable
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn registry_rejects_malformed_builtin_optional_permissions_before_probe() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("malformed-builtin-permission-ran.marker");
        let script = format!("touch {}", marker.display());
        let mut manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", &script],
            1024,
            1024,
            5000,
        );
        manifest_value["permissions"]["secrets"] = json!(42);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [manifest(manifest_value)],
            &[],
        );
        let conn = conn();

        let error = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap_err()
            .to_string();

        assert!(error.contains("is not registered"), "{error}");
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(
            registry.diagnostics()[0]
                .message
                .contains("permissions.secrets"),
            "{:?}",
            registry.diagnostics()
        );
        assert!(!marker.exists(), "malformed built-in permission probe ran");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn registry_rejects_malformed_repository_optional_permissions_before_probe() {
        let dir = tempdir().unwrap();
        let marker = dir
            .path()
            .join("malformed-repository-permission-ran.marker");
        let script = format!("touch {}", marker.display());
        let mut manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", &script],
            1024,
            1024,
            5000,
        );
        manifest_value["permissions"]["environment"] = json!("");
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let error = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap_err()
            .to_string();

        assert!(error.contains("is not registered"), "{error}");
        assert_eq!(
            registry.diagnostics()[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestMismatch
        );
        assert!(
            registry.diagnostics()[0]
                .message
                .contains("permissions.environment"),
            "{:?}",
            registry.diagnostics()
        );
        assert!(
            !marker.exists(),
            "malformed repository permission probe ran"
        );
        let manifest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_manifests",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let instance_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(manifest_count, 0);
        assert_eq!(instance_count, 0);
    }

    #[test]
    fn registry_valid_optional_permissions_round_trip_exactly() {
        let dir = tempdir().unwrap();
        let mut manifest_value = manifest_value("cargo", vec!["--version"], 1024);
        manifest_value["permissions"]["environment"] = json!("read_env:PATH");
        manifest_value["permissions"]["secrets"] = json!("none");
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(instance.permissions.network, "none");
        assert_eq!(instance.permissions.filesystem, "read_workspace");
        assert_eq!(
            instance.permissions.environment.as_deref(),
            Some("read_env:PATH")
        );
        assert_eq!(instance.permissions.secrets.as_deref(), Some("none"));
    }

    #[cfg(unix)]
    #[test]
    fn registry_path_executable_resolution_is_bound_to_runtime_identity() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let bin_a = dir.path().join("bin-a");
        let bin_b = dir.path().join("bin-b");
        fs::create_dir_all(&bin_a).unwrap();
        fs::create_dir_all(&bin_b).unwrap();
        let marker_a = dir.path().join("selected-a.marker");
        let marker_b = dir.path().join("selected-b.marker");
        let command_a = bin_a.join("planr-path-probe");
        let command_b = bin_b.join("planr-path-probe");
        fs::write(
            &command_a,
            format!("#!/bin/sh\n: > {}\n", marker_a.display()),
        )
        .unwrap();
        fs::write(
            &command_b,
            format!("#!/bin/sh\n: > {}\n", marker_b.display()),
        )
        .unwrap();
        fs::set_permissions(&command_a, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&command_b, fs::Permissions::from_mode(0o755)).unwrap();
        let path_a = bin_a.to_string_lossy().to_string();
        let path_b = bin_b.to_string_lossy().to_string();
        let manifest_value = manifest_value("planr-path-probe", vec![], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let instance_a = with_captured_path_env_override(path_a, || {
            registry
                .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
                .unwrap()
        });
        let instance_b = with_captured_path_env_override(path_b, || {
            registry
                .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
                .unwrap()
        });

        assert_eq!(
            instance_a.availability.status,
            CapabilityAvailabilityStatus::Available,
            "{:?}",
            instance_a.availability.reason
        );
        assert_eq!(
            instance_b.availability.status,
            CapabilityAvailabilityStatus::Available,
            "{:?}",
            instance_b.availability.reason
        );
        assert!(marker_a.exists(), "first PATH executable was not selected");
        assert!(marker_b.exists(), "second PATH executable was not selected");
        assert_ne!(
            instance_a.environment.digest.as_str(),
            instance_b.environment.digest.as_str(),
            "PATH/executable drift reused the same environment digest"
        );
        assert_eq!(
            registry.runtime_availability.len(),
            2,
            "PATH/executable drift reused the same runtime availability key"
        );
        let stored_instances: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances WHERE manifest_id = 'vcap-test-process-v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_instances, 2);
    }

    #[test]
    fn registry_windows_candidate_generation_preserves_command_suffix_search() {
        let candidates = windows_probe_executable_candidates(
            Path::new("bin"),
            "cargo",
            Some(".CMD;.BAT;.EXE".to_string()),
        );
        let file_names = candidates
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(file_names, vec!["cargo.EXE", "cargo.CMD", "cargo.BAT"]);

        let with_extension = windows_probe_executable_candidates(
            Path::new("bin"),
            "cargo.exe",
            Some(".CMD;.BAT".to_string()),
        );
        let file_names = with_extension
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(file_names, vec!["cargo.exe"]);
    }

    #[cfg(windows)]
    #[test]
    fn registry_windows_resolution_prefers_exe_over_extensionless_sibling() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("planr-sibling-probe"), b"extensionless").unwrap();
        fs::write(dir.path().join("planr-sibling-probe.exe"), b"exe").unwrap();

        let resolved =
            resolve_probe_executable("planr-sibling-probe", Some(&dir.path().to_string_lossy()))
                .unwrap();

        assert_eq!(
            resolved.path.file_name().unwrap().to_string_lossy(),
            "planr-sibling-probe.exe"
        );
        assert_eq!(resolved.content_digest, sha256_prefixed_bytes(b"exe"));
    }

    #[cfg(windows)]
    #[test]
    fn registry_windows_extensionless_probe_resolves_suffixed_path_candidate() {
        let dir = tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let marker = dir.path().join("selected-windows.marker");
        let command = bin.join("planr-windows-probe.cmd");
        fs::write(
            &command,
            format!("@echo off\r\ntype nul > \"{}\"\r\n", marker.display()),
        )
        .unwrap();
        let manifest_value = manifest_value("planr-windows-probe", vec![], 1024);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let instance = with_captured_path_env_override(bin.to_string_lossy().to_string(), || {
            registry
                .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
                .unwrap()
        });

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Available,
            "{:?}",
            instance.availability.reason
        );
        assert!(
            marker.exists(),
            "suffixed Windows PATH candidate did not run"
        );
    }

    #[test]
    fn registry_probe_times_out_through_bounded_process_owner() {
        let dir = tempdir().unwrap();
        let manifest_value =
            manifest_value_with_id("vcap-test-process-v1", "sleep", vec!["1"], 1024, 1024, 20);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(instance.probe_result.outcome, AttemptStatus::TimedOut);
        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Unavailable
        );
    }

    #[cfg(unix)]
    #[test]
    fn registry_probe_cleans_up_descendants_through_bounded_process_owner() {
        let dir = tempdir().unwrap();
        let pid_path = dir.path().join("grandchild.pid");
        let script = format!(
            "sh -c 'echo $$ > {}; trap \"\" TERM; while :; do sleep 1; done' & while [ ! -s {} ]; do sleep 0.01; done",
            pid_path.display(),
            pid_path.display()
        );
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", &script],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let started = Instant::now();

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(instance.probe_result.outcome, AttemptStatus::Passed);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "registry probe blocked on descendant-held pipes"
        );
        assert_process_is_gone(read_published_pid(&pid_path));
    }

    #[test]
    fn registry_consecutive_probes_store_fresh_instances_and_availability_transitions() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", "test -f available.flag"],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let failed = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        fs::write(dir.path().join("available.flag"), "ready").unwrap();
        let passed = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_ne!(
            failed.probe_result.probe_execution_id.as_str(),
            passed.probe_result.probe_execution_id.as_str()
        );
        assert_eq!(failed.probe_result.outcome, AttemptStatus::Failed);
        assert_eq!(passed.probe_result.outcome, AttemptStatus::Passed);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances WHERE manifest_id = 'vcap-test-process-v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                .is_empty()
        );
        assert_eq!(
            diagnostic_count(
                &registry,
                CapabilityRegistryDiagnosticCode::ProbeUnavailable
            ),
            0,
            "same-context successful probe left a stale probe diagnostic"
        );
    }

    #[cfg(unix)]
    #[test]
    fn registry_permission_denied_to_pass_clears_current_runtime_diagnostic() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let denied = dir.path().join("denied-adapter");
        fs::write(&denied, "#!/bin/sh\necho recovered\n").unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o644)).unwrap();
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", "./denied-adapter"],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let registration = registration_for_manifest(&digest, &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration],
        );
        let conn = conn();

        let denied = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();
        fs::set_permissions(
            dir.path().join("denied-adapter"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let passed = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            denied.availability.status,
            CapabilityAvailabilityStatus::PermissionDenied
        );
        assert_eq!(
            passed.availability.status,
            CapabilityAvailabilityStatus::Available
        );
        assert_eq!(
            diagnostic_count(
                &registry,
                CapabilityRegistryDiagnosticCode::PermissionDenied
            ),
            0,
            "same-context successful probe left a stale permission diagnostic"
        );
        assert!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                .is_empty()
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances WHERE manifest_id = 'vcap-test-process-v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn registry_runtime_probe_failures_replace_per_key_and_recovery_keeps_other_keys() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", "test -f available.flag"],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let env_a = CapabilityRuntimeContext {
            environment_id: "env-a",
            ..runtime()
        };
        let env_b = CapabilityRuntimeContext {
            environment_id: "env-b",
            ..runtime()
        };
        registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", env_a)
            .unwrap();
        registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", env_a)
            .unwrap();
        assert_eq!(
            diagnostic_count(
                &registry,
                CapabilityRegistryDiagnosticCode::ProbeUnavailable
            ),
            1,
            "repeated same-key failures duplicated current diagnostics"
        );

        registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", env_b)
            .unwrap();
        assert_eq!(
            diagnostic_count(
                &registry,
                CapabilityRegistryDiagnosticCode::ProbeUnavailable
            ),
            2,
            "different runtime keys should keep independent current diagnostics"
        );

        fs::write(dir.path().join("available.flag"), "ready").unwrap();
        let passed = registry
            .probe_and_store(&conn, dir.path(), "vcap-test-process-v1", env_a)
            .unwrap();

        assert_eq!(
            passed.availability.status,
            CapabilityAvailabilityStatus::Available
        );
        assert_eq!(
            diagnostic_count(
                &registry,
                CapabilityRegistryDiagnosticCode::ProbeUnavailable
            ),
            1,
            "same-key recovery should not clear another runtime key's current failure"
        );
    }

    #[test]
    fn registry_runtime_availability_isolated_by_environment_after_storage() {
        let dir = tempdir().unwrap();
        let manifest_value = manifest_value_with_id(
            "vcap-test-process-v1",
            "sh",
            vec!["-c", "test \"$PLANR_ENV_READY\" = yes"],
            1024,
            1024,
            5000,
        );
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );
        let conn = conn();

        let failed = registry
            .probe_and_store(
                &conn,
                dir.path(),
                "vcap-test-process-v1",
                CapabilityRuntimeContext {
                    environment_id: "env-a",
                    ..runtime()
                },
            )
            .unwrap();
        let capability = registry
            .capabilities
            .get("vcap-test-process-v1")
            .unwrap()
            .clone();
        let passed_key = RuntimeAvailabilityKey::new(
            &capability,
            &ValidatedCapabilityRuntimeContext::parse(CapabilityRuntimeContext {
                environment_id: "env-b",
                ..runtime()
            })
            .unwrap(),
            &registry.repository_root_digest,
            &ProbeCommandResolution::capture(&capability.manifest.availability_probe.execution),
        )
        .unwrap();
        registry
            .runtime_availability
            .insert(passed_key, CapabilityAvailabilityStatus::Available);

        assert_eq!(
            failed.availability.status,
            CapabilityAvailabilityStatus::ProbeFailed
        );
        assert!(
            registry
                .available_diagnostics_for_declared_observations(["planr.test.output".to_string()])
                .is_empty(),
            "an available env-b runtime should not be overwritten by failed env-a"
        );
    }

    #[test]
    fn registry_probe_honors_asymmetric_stdout_and_stderr_limits() {
        let dir = tempdir().unwrap();
        let script = "printf abcdef; printf x >&2";
        let manifest_value =
            manifest_value_with_id("vcap-test-process-v1", "sh", vec!["-c", script], 8, 1, 5000);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Available
        );
    }

    #[test]
    fn registry_probe_rejects_stream_that_exceeds_its_own_limit_only() {
        let dir = tempdir().unwrap();
        let script = "printf abcdef; printf xy >&2";
        let manifest_value =
            manifest_value_with_id("vcap-test-process-v1", "sh", vec!["-c", script], 8, 1, 5000);
        let digest = write_manifest(dir.path(), &manifest_value);
        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            dir.path(),
            [],
            &[registration_for_manifest(&digest, &manifest_value)],
        );

        let instance = registry
            .probe_and_store(&conn(), dir.path(), "vcap-test-process-v1", runtime())
            .unwrap();

        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::ProbeFailed
        );
        assert!(
            instance
                .availability
                .reason
                .unwrap()
                .contains("output exceeded")
        );
    }

    fn runtime() -> CapabilityRuntimeContext<'static> {
        CapabilityRuntimeContext {
            host: "codex",
            surface: "local-process",
            host_version: "test",
            environment_id: "env-test",
        }
    }

    #[cfg(unix)]
    fn read_published_pid(pid_path: &Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match fs::read_to_string(pid_path) {
                Ok(content) if !content.trim().is_empty() => {
                    return content.trim().parse::<i32>().unwrap();
                }
                _ if Instant::now() >= deadline => {
                    panic!("descendant did not publish pid at {}", pid_path.display());
                }
                _ => thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    #[cfg(unix)]
    fn assert_process_is_gone(pid: i32) {
        for _ in 0..20 {
            let status = StdCommand::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .stderr(StdStdio::null())
                .status()
                .unwrap();
            if !status.success() {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("descendant process {pid} survived registry probe cleanup");
    }
}
