#![allow(dead_code)]

use super::model::{
    EvidenceDomainError, EvidenceId, EvidencePolicy, EvidenceReceipt, EvidenceScope,
    EvidenceWaiver, ObservationRequirement, Sha256Digest, SourceBinding, TrustedPolicySource,
    TrustedReceiptBinding,
};
use crate::canonical_json::{
    sha256_json_digest, sha256_json_digest_without_top_level_field, sha256_prefixed_bytes,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const RESERVED_EXTENSION_NAMESPACES: &[&str] = &["planr", "mcp", "host"];

#[derive(Debug, Clone)]
pub(crate) struct EvidencePolicyDocument {
    pub policy: EvidencePolicy,
    pub waivers: Vec<EvidenceWaiver>,
    pub digest: String,
    trusted_builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpandedProofObservation {
    pub derived_id: String,
    pub preset_id: String,
    pub observation_id: String,
    pub observation_type: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EvidencePolicyLayerMaterial {
    pub value: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct EvidencePolicyResolutionContext {
    pub evaluated_at: OffsetDateTime,
    pub source: super::model::SourceBinding,
    pub target: super::model::TargetBinding,
    pub observation_ids: BTreeSet<String>,
    pub selected_scopes: SelectedPolicyScopes,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SelectedPolicyScopes {
    pub goal_id: Option<String>,
    pub plan_id: Option<String>,
    pub item_id: Option<String>,
    pub criterion_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenPolicyLayer {
    scope: EvidenceScope,
    policy_digest: Sha256Digest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedPolicyLayerMaterial {
    policy_digest: Sha256Digest,
    #[serde(default)]
    preset_id: Option<EvidenceId>,
    #[serde(default)]
    pub binding: Option<bool>,
    #[serde(default)]
    pub fixtures_allowed: Option<bool>,
    #[serde(default)]
    pub mocks_allowed: Option<bool>,
    #[serde(default)]
    pub max_age_seconds: Option<i64>,
    #[serde(default)]
    pub assurance_level: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedEvidencePolicyLayer {
    pub scope_kind: String,
    pub scope_id: String,
    pub preset_id: String,
    pub binding: bool,
    pub fixtures_allowed: bool,
    pub mocks_allowed: bool,
    pub max_age_seconds: i64,
    pub assurance_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidencePolicyDiagnostic {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidencePolicyDiagnostics {
    pub diagnostics: Vec<EvidencePolicyDiagnostic>,
}

impl std::fmt::Display for EvidencePolicyDiagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let joined = self
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.path, diagnostic.message))
            .collect::<Vec<_>>()
            .join("; ");
        write!(f, "{joined}")
    }
}

impl std::error::Error for EvidencePolicyDiagnostics {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryPolicyBinding {
    pub digest: Sha256Digest,
    pub source: TrustedPolicySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceRepositorySnapshot {
    pub source: SourceBinding,
    pub policy: Option<RepositoryPolicyBinding>,
}

impl EvidenceRepositorySnapshot {
    pub(crate) fn trusted_policy_binding(&self) -> anyhow::Result<RepositoryPolicyBinding> {
        self.policy
            .clone()
            .ok_or_else(|| anyhow::anyhow!("repository Evidence policy is required for execution"))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvidencePolicyDocument {
    #[serde(flatten)]
    policy: EvidencePolicy,
    #[serde(default)]
    waivers: Vec<EvidenceWaiver>,
}

pub(crate) fn parse_evidence_policy_yaml(
    text: &str,
) -> Result<EvidencePolicyDocument, EvidencePolicyDiagnostics> {
    parse_evidence_policy_yaml_with_owner(text, false)
}

pub(crate) fn parse_trusted_builtin_evidence_policy_yaml(
    text: &str,
) -> Result<EvidencePolicyDocument, EvidencePolicyDiagnostics> {
    parse_evidence_policy_yaml_with_owner(text, true)
}

pub(crate) fn capture_repository_snapshot(
    repository_root: &Path,
) -> Result<EvidenceRepositorySnapshot, EvidencePolicyDiagnostics> {
    Ok(EvidenceRepositorySnapshot {
        source: capture_source_binding(repository_root)?,
        policy: load_repository_policy_binding(repository_root)?,
    })
}

pub(crate) fn load_repository_policy_binding(
    repository_root: &Path,
) -> Result<Option<RepositoryPolicyBinding>, EvidencePolicyDiagnostics> {
    let policy_path = repository_root.join(".planr/evidence.yaml");
    if !policy_path.exists() {
        return Ok(None);
    }
    let policy_yaml = fs::read_to_string(&policy_path).map_err(|error| {
        diagnostics(vec![diag(
            ".planr/evidence.yaml",
            format!("must be readable: {error}"),
        )])
    })?;
    let document = parse_evidence_policy_yaml(&policy_yaml)?;
    let digest = Sha256Digest::parse(document.digest)
        .map_err(|error| diagnostics(vec![diag("policy_digest", error.to_string())]))?;
    Ok(Some(RepositoryPolicyBinding {
        digest,
        source: TrustedPolicySource::Repository,
    }))
}

pub(crate) fn parse_trusted_receipt_binding(
    trusted_binding_json: &str,
    receipt: &EvidenceReceipt,
) -> Result<TrustedReceiptBinding, EvidenceDomainError> {
    let binding: TrustedReceiptBinding = serde_json::from_str(trusted_binding_json)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("trusted_binding_json"))?;
    binding.validate_receipt_exact(receipt)?;
    Ok(binding)
}

pub(crate) fn trusted_receipt_binding_value(
    receipt: &EvidenceReceipt,
    policy: RepositoryPolicyBinding,
) -> Result<Value, EvidenceDomainError> {
    let binding = TrustedReceiptBinding::from_receipt(receipt, policy.digest, policy.source);
    serde_json::to_value(binding)
        .map_err(|_| EvidenceDomainError::InvalidTrustedBinding("trusted_binding_json"))
}

fn capture_source_binding(
    repository_root: &Path,
) -> Result<SourceBinding, EvidencePolicyDiagnostics> {
    let revision = git_stdout(
        repository_root,
        &["rev-parse", "--verify", "HEAD"],
        "source.revision",
    )?;
    let head_tree = git_stdout(
        repository_root,
        &["rev-parse", "HEAD^{tree}"],
        "source.tree_digest",
    )?;
    let index = source_git_stdout(repository_root, &["ls-files", "-s"], "source.tree_digest")?;
    let status = source_git_stdout(
        repository_root,
        &["status", "--porcelain=v1"],
        "source.dirty",
    )?;
    let diff = source_git_stdout(
        repository_root,
        &["diff", "--binary", "HEAD"],
        "source.tree_digest",
    )?;
    let diff_cached = source_git_stdout(
        repository_root,
        &["diff", "--cached", "--binary"],
        "source.tree_digest",
    )?;
    let untracked = untracked_file_digests(repository_root)?;
    let dirty = !status.trim().is_empty();
    let tree_digest = sha256_json_digest(&json!({
        "revision": revision,
        "head_tree": head_tree,
        "index": index,
        "status": status,
        "diff": diff,
        "diff_cached": diff_cached,
        "untracked": untracked,
    }))
    .map_err(|error| diagnostics(vec![diag("source.tree_digest", error.to_string())]))?;
    Ok(SourceBinding {
        revision,
        tree_digest: Sha256Digest::parse(tree_digest)
            .map_err(|error| diagnostics(vec![diag("source.tree_digest", error.to_string())]))?,
        dirty,
    })
}

fn untracked_file_digests(repository_root: &Path) -> Result<Vec<Value>, EvidencePolicyDiagnostics> {
    let files = source_git_stdout(
        repository_root,
        &["ls-files", "--others", "--exclude-standard"],
        "source.tree_digest",
    )?;
    files
        .lines()
        .filter(|line| !line.is_empty())
        .map(|relative| {
            if Path::new(relative).is_absolute()
                || Path::new(relative)
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(diagnostics(vec![diag(
                    "source.tree_digest",
                    format!("git reported uncontained untracked path {relative}"),
                )]));
            }
            let bytes = fs::read(repository_root.join(relative)).map_err(|error| {
                diagnostics(vec![diag(
                    "source.tree_digest",
                    format!("untracked file {relative} must be readable: {error}"),
                )])
            })?;
            Ok(json!({
                "path": relative,
                "digest": sha256_prefixed_bytes(&bytes),
            }))
        })
        .collect()
}

pub(crate) const SOURCE_PATHS: &[&str] = &[
    ".",
    ":(exclude).planr/planr.sqlite",
    ":(exclude).planr/planr.sqlite-shm",
    ":(exclude).planr/planr.sqlite-wal",
    ":(exclude).planr/artifacts/**",
    ":(exclude).planr/verification/**",
    ":(exclude).planr/evidence/runs/**",
    ":(exclude).planr/evidence/attempts/**",
    ":(exclude).planr/evidence/receipts/**",
    ":(exclude).planr/evidence/coverage/**",
];

fn source_git_stdout(
    repository_root: &Path,
    args: &[&str],
    field: &'static str,
) -> Result<String, EvidencePolicyDiagnostics> {
    let mut scoped = Vec::with_capacity(args.len() + SOURCE_PATHS.len() + 1);
    scoped.extend_from_slice(args);
    scoped.push("--");
    scoped.extend_from_slice(SOURCE_PATHS);
    git_stdout(repository_root, &scoped, field)
}

fn git_stdout(
    repository_root: &Path,
    args: &[&str],
    field: &'static str,
) -> Result<String, EvidencePolicyDiagnostics> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(args)
        .output()
        .map_err(|error| {
            diagnostics(vec![diag(
                field,
                format!("git command failed to start: {error}"),
            )])
        })?;
    if !output.status.success() {
        return Err(diagnostics(vec![diag(
            field,
            format!(
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )]));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

fn parse_evidence_policy_yaml_with_owner(
    text: &str,
    trusted_builtin: bool,
) -> Result<EvidencePolicyDocument, EvidencePolicyDiagnostics> {
    let value = serde_yaml::from_str::<serde_yaml::Value>(text).map_err(|error| {
        diagnostics(vec![EvidencePolicyDiagnostic {
            path: ".planr/evidence.yaml".to_string(),
            message: format!("YAML parse error: {error}"),
        }])
    })?;
    let json_value = serde_json::to_value(value).map_err(|error| {
        diagnostics(vec![EvidencePolicyDiagnostic {
            path: ".planr/evidence.yaml".to_string(),
            message: format!("YAML value cannot be represented as JSON: {error}"),
        }])
    })?;
    let raw = serde_json::from_value::<RawEvidencePolicyDocument>(json_value.clone()).map_err(
        |error| {
            diagnostics(vec![EvidencePolicyDiagnostic {
                path: ".planr/evidence.yaml".to_string(),
                message: format!("Evidence policy contract violation: {error}"),
            }])
        },
    )?;
    let digest = policy_digest_preimage(&json_value)
        .and_then(|value| sha256_json_digest(&value))
        .map_err(|error| {
            diagnostics(vec![EvidencePolicyDiagnostic {
                path: "policy_digest".to_string(),
                message: error.to_string(),
            }])
        })?;

    let document = EvidencePolicyDocument {
        policy: raw.policy,
        waivers: raw.waivers,
        digest,
        trusted_builtin,
    };
    validate_evidence_policy_document(&document)?;
    Ok(document)
}

pub(crate) fn validate_repository_observation_schemas(
    repository_root: &Path,
    document: &EvidencePolicyDocument,
) -> Result<(), EvidencePolicyDiagnostics> {
    let mut diagnostics = Vec::new();
    let schema_root = repository_root.join(".planr/evidence/schemas");
    for (index, registration) in document
        .policy
        .observation_schema_registrations
        .iter()
        .enumerate()
    {
        let path = format!("observation_schema_registrations[{index}]");
        let Some(schema_ref) = string_at(registration, &["schema_ref"]) else {
            diagnostics.push(diag(format!("{path}.schema_ref"), "is required"));
            continue;
        };
        let Some(expected_digest) = string_at(registration, &["schema_digest"]) else {
            diagnostics.push(diag(format!("{path}.schema_digest"), "is required"));
            continue;
        };
        let schema_path = match repository_schema_path(&schema_root, schema_ref) {
            Ok(schema_path) => schema_path,
            Err(message) => {
                diagnostics.push(diag(format!("{path}.schema_ref"), message));
                continue;
            }
        };
        let schema_text = match fs::read_to_string(&schema_path) {
            Ok(schema_text) => schema_text,
            Err(error) => {
                diagnostics.push(diag(
                    format!("{path}.schema_ref"),
                    format!(
                        "schema file {} must be readable: {error}",
                        schema_path.display()
                    ),
                ));
                continue;
            }
        };
        let schema_value = match serde_json::from_str::<Value>(&schema_text) {
            Ok(schema_value) => schema_value,
            Err(error) => {
                diagnostics.push(diag(
                    format!("{path}.schema_ref"),
                    format!(
                        "schema file {} must contain JSON: {error}",
                        schema_path.display()
                    ),
                ));
                continue;
            }
        };
        if string_at(&schema_value, &["$id"]).is_some_and(|id| id != schema_ref) {
            diagnostics.push(diag(
                format!("{path}.schema_ref"),
                "schema file $id must match schema_ref",
            ));
        }
        match sha256_json_digest(&schema_value) {
            Ok(actual_digest) if actual_digest == expected_digest => {}
            Ok(actual_digest) => diagnostics.push(diag(
                format!("{path}.schema_digest"),
                format!("must equal repository schema digest {actual_digest}"),
            )),
            Err(error) => diagnostics.push(diag(
                format!("{path}.schema_digest"),
                format!(
                    "schema file {} must be digestible: {error}",
                    schema_path.display()
                ),
            )),
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(EvidencePolicyDiagnostics { diagnostics })
    }
}

pub(crate) fn load_repository_observation_schema(
    repository_root: &Path,
    schema_ref: &str,
) -> Result<Option<Value>, EvidencePolicyDiagnostics> {
    let policy_path = repository_root.join(".planr/evidence.yaml");
    if !policy_path.exists() {
        return Ok(None);
    }
    let policy_yaml = fs::read_to_string(&policy_path).map_err(|error| {
        diagnostics(vec![diag(
            ".planr/evidence.yaml",
            format!("must be readable: {error}"),
        )])
    })?;
    let document = parse_evidence_policy_yaml(&policy_yaml)?;
    let Some(registration) = document
        .policy
        .observation_schema_registrations
        .iter()
        .find(|registration| string_at(registration, &["schema_ref"]) == Some(schema_ref))
    else {
        return Ok(None);
    };
    let Some(expected_digest) = string_at(registration, &["schema_digest"]) else {
        return Err(diagnostics(vec![diag(
            "observation_schema_registrations.schema_digest",
            "is required",
        )]));
    };
    let schema_path =
        repository_schema_path(&repository_root.join(".planr/evidence/schemas"), schema_ref)
            .map_err(|message| {
                diagnostics(vec![diag(
                    "observation_schema_registrations.schema_ref",
                    message,
                )])
            })?;
    let schema_text = fs::read_to_string(&schema_path).map_err(|error| {
        diagnostics(vec![diag(
            "observation_schema_registrations.schema_ref",
            format!(
                "schema file {} must be readable: {error}",
                schema_path.display()
            ),
        )])
    })?;
    let schema_value = serde_json::from_str::<Value>(&schema_text).map_err(|error| {
        diagnostics(vec![diag(
            "observation_schema_registrations.schema_ref",
            format!(
                "schema file {} must contain JSON: {error}",
                schema_path.display()
            ),
        )])
    })?;
    let validation_schema = schema_value
        .get("json_schema")
        .cloned()
        .unwrap_or_else(|| schema_value.clone());
    match sha256_json_digest(&schema_value) {
        Ok(actual_digest) if actual_digest == expected_digest => Ok(Some(validation_schema)),
        Ok(actual_digest) => Err(diagnostics(vec![diag(
            "observation_schema_registrations.schema_digest",
            format!("must equal repository schema digest {actual_digest}"),
        )])),
        Err(error) => Err(diagnostics(vec![diag(
            "observation_schema_registrations.schema_digest",
            format!(
                "schema file {} must be digestible: {error}",
                schema_path.display()
            ),
        )])),
    }
}

fn repository_schema_path(schema_root: &Path, schema_ref: &str) -> Result<PathBuf, String> {
    let schema_identifier = schema_ref.strip_prefix("schema://").unwrap_or(schema_ref);
    if schema_identifier.is_empty() {
        return Err("schema_ref must identify a repository schema".to_string());
    }
    if schema_identifier.contains('/') {
        return Err("schema_ref must not contain path separators".to_string());
    }
    if !schema_identifier
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@'))
    {
        return Err("schema_ref must be a repository schema identifier".to_string());
    }
    Ok(schema_root.join(format!(
        "{}.schema.json",
        schema_identifier.replace('@', ".")
    )))
}

pub(crate) fn validate_evidence_policy_document(
    document: &EvidencePolicyDocument,
) -> Result<(), EvidencePolicyDiagnostics> {
    let mut diagnostics = Vec::new();
    validate_policy_digest(document, &mut diagnostics);
    validate_presets(document, &mut diagnostics);
    validate_schema_registrations(document, &mut diagnostics);
    validate_adapter_registrations(document, &mut diagnostics);
    validate_policy_sections(document, &mut diagnostics);
    validate_waivers(document, &mut diagnostics);
    if let Err(error) = expand_proof_presets(document) {
        diagnostics.extend(error.diagnostics);
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(EvidencePolicyDiagnostics { diagnostics })
    }
}

pub(crate) fn expand_proof_presets(
    document: &EvidencePolicyDocument,
) -> Result<Vec<ExpandedProofObservation>, EvidencePolicyDiagnostics> {
    let mut diagnostics = Vec::new();
    let mut expanded = document
        .policy
        .named_presets
        .iter()
        .flat_map(|preset| {
            preset
                .observations
                .iter()
                .map(|observation| ExpandedProofObservation {
                    derived_id: format!("{}:{}", preset.id.as_str(), observation.id.as_str()),
                    preset_id: preset.id.as_str().to_string(),
                    observation_id: observation.id.as_str().to_string(),
                    observation_type: observation.observation_type.as_str().to_string(),
                })
        })
        .collect::<Vec<_>>();
    expanded.sort_by(|left, right| {
        left.derived_id
            .cmp(&right.derived_id)
            .then_with(|| left.observation_type.cmp(&right.observation_type))
    });
    let mut seen = BTreeSet::new();
    for observation in &expanded {
        if !seen.insert(observation.derived_id.clone()) {
            diagnostics.push(diag(
                "named_presets.observations",
                format!(
                    "duplicate derived proof observation id {}",
                    observation.derived_id
                ),
            ));
        }
    }
    if diagnostics.is_empty() {
        Ok(expanded)
    } else {
        Err(EvidencePolicyDiagnostics { diagnostics })
    }
}

pub(crate) fn resolve_policy_layers(
    document: &EvidencePolicyDocument,
    layer_policies: &[EvidencePolicyLayerMaterial],
    context: &EvidencePolicyResolutionContext,
) -> Result<Vec<ResolvedEvidencePolicyLayer>, EvidencePolicyDiagnostics> {
    let mut diagnostics = Vec::new();
    let preset_observations = preset_observations(document, &mut diagnostics);
    let preset_ids = preset_observations.keys().cloned().collect::<BTreeSet<_>>();
    let layers = typed_frozen_policy_layers(document, &mut diagnostics);
    validate_scope_chain(&layers, &mut diagnostics);
    validate_selected_scope_chain(&layers, context, &mut diagnostics);
    let material_by_digest = typed_policy_materials(layer_policies, &mut diagnostics);
    let mut ordered = layers.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|layer| scope_rank(&layer.scope.kind));
    let mut current = base_policy_state(document, &mut diagnostics);
    let mut resolved = Vec::new();
    for layer in ordered {
        if !layer_matches_selected_scope(layer, context) {
            continue;
        }
        let Some(material) = material_by_digest.get(layer.policy_digest.as_str()) else {
            diagnostics.push(diag(
                format!("layers.{}:{}", layer.scope.kind, layer.scope.id.as_str()),
                format!(
                    "missing policy material for {}",
                    layer.policy_digest.as_str()
                ),
            ));
            continue;
        };
        let mut next = current.clone();
        if let Some(preset_id) = &material.preset_id {
            next.preset_id = preset_id.as_str().to_string();
        }
        if let Some(binding) = material.binding {
            next.binding = binding;
        }
        if let Some(fixtures_allowed) = material.fixtures_allowed {
            next.fixtures_allowed = fixtures_allowed;
        }
        if let Some(mocks_allowed) = material.mocks_allowed {
            next.mocks_allowed = mocks_allowed;
        }
        if let Some(max_age_seconds) = material.max_age_seconds {
            next.max_age_seconds = max_age_seconds;
        }
        if let Some(assurance_level) = &material.assurance_level {
            next.assurance_level = assurance_level.clone();
        }
        if !preset_ids.contains(&next.preset_id) {
            diagnostics.push(diag(
                format!(
                    "layers.{}:{}.preset_id",
                    layer.scope.kind,
                    layer.scope.id.as_str()
                ),
                "must reference a declared proof preset",
            ));
        }
        let weakened_observations = weakened_observations(&current, &next, &preset_observations);
        let weakening = weakening_fields(&current, &next, !weakened_observations.is_empty());
        if !weakening.is_empty()
            && !waiver_covers_layer(document, layer, context, &weakened_observations)
        {
            diagnostics.push(diag(
                format!("layers.{}:{}", layer.scope.kind, layer.scope.id.as_str()),
                format!(
                    "weakening requires matching waiver for {}",
                    weakening.join(",")
                ),
            ));
        }
        current = next;
        resolved.push(ResolvedEvidencePolicyLayer {
            scope_kind: layer.scope.kind.clone(),
            scope_id: layer.scope.id.as_str().to_string(),
            preset_id: current.preset_id.clone(),
            binding: current.binding,
            fixtures_allowed: current.fixtures_allowed,
            mocks_allowed: current.mocks_allowed,
            max_age_seconds: current.max_age_seconds,
            assurance_level: current.assurance_level.clone(),
        });
    }
    if diagnostics.is_empty() {
        Ok(resolved)
    } else {
        Err(EvidencePolicyDiagnostics { diagnostics })
    }
}

fn validate_policy_digest(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    if document.policy.policy_digest.as_str() != document.digest {
        diagnostics.push(EvidencePolicyDiagnostic {
            path: "policy_digest".to_string(),
            message: format!(
                "must equal canonical policy digest {}, got {}",
                document.digest,
                document.policy.policy_digest.as_str()
            ),
        });
    }
}

fn policy_digest_preimage(value: &Value) -> anyhow::Result<Value> {
    let mut value = value.clone();
    let Some(object) = value.as_object_mut() else {
        anyhow::bail!("evidence policy must be a YAML mapping");
    };
    object.remove("policy_digest");
    object.remove("waivers");
    Ok(value)
}

fn validate_presets(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    let presets = &document.policy.named_presets;
    if presets.is_empty() {
        diagnostics.push(diag("named_presets", "must contain at least one preset"));
        return;
    }

    let mut preset_ids = BTreeSet::new();
    let mut seen_observation_ids = BTreeSet::new();
    for (preset_index, preset) in presets.iter().enumerate() {
        let preset_path = format!("named_presets[{preset_index}]");
        if !preset_ids.insert(preset.id.as_str().to_string()) {
            diagnostics.push(diag(
                format!("{preset_path}.id"),
                "must be unique across proof_presets",
            ));
        }
        if !declared_extension_namespace(document, preset.namespace.as_str()) {
            diagnostics.push(diag(
                format!("{preset_path}.namespace"),
                "must be declared in extension_namespaces",
            ));
        }
        if preset.observations.is_empty() {
            diagnostics.push(diag(
                format!("{preset_path}.observations"),
                "must contain at least one observation",
            ));
        }
        for (observation_index, observation) in preset.observations.iter().enumerate() {
            let observation_path = format!("{preset_path}.observations[{observation_index}]");
            let derived_observation_id =
                format!("{}:{}", preset.id.as_str(), observation.id.as_str());
            if !seen_observation_ids.insert(derived_observation_id) {
                diagnostics.push(diag(
                    format!("{observation_path}.id"),
                    "must be unique across deterministic proof preset expansion",
                ));
            }
            if !type_is_owned_by_namespace(
                observation.observation_type.as_str(),
                preset.namespace.as_str(),
            ) {
                diagnostics.push(diag(
                    format!("{observation_path}.type"),
                    "must be owned by the preset namespace",
                ));
            }
        }
    }

    let default_preset_id = string_at(&document.policy.defaults, &["preset_id"]);
    if default_preset_id.is_none_or(|id| !preset_ids.contains(id)) {
        diagnostics.push(diag(
            "defaults.preset_id",
            "must reference a declared proof preset",
        ));
    }
}

fn validate_schema_registrations(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    if document.policy.observation_schema_registrations.is_empty() {
        diagnostics.push(diag(
            "observation_schema_registrations",
            "must contain at least one registration",
        ));
        return;
    }

    let mut schemas_by_type = BTreeMap::new();
    for (index, registration) in document
        .policy
        .observation_schema_registrations
        .iter()
        .enumerate()
    {
        let path = format!("observation_schema_registrations[{index}]");
        let Some(observation_type) = string_at(registration, &["type"]) else {
            diagnostics.push(diag(format!("{path}.type"), "is required"));
            continue;
        };
        let Some(owning_namespace) = string_at(registration, &["owning_namespace"]) else {
            diagnostics.push(diag(format!("{path}.owning_namespace"), "is required"));
            continue;
        };
        if !document.trusted_builtin && is_reserved_namespace(owning_namespace) {
            diagnostics.push(diag(
                format!("{path}.owning_namespace"),
                "uses a reserved namespace",
            ));
        }
        if !declared_extension_namespace(document, owning_namespace) {
            diagnostics.push(diag(
                format!("{path}.owning_namespace"),
                "must be declared in extension_namespaces",
            ));
        }
        if !type_is_owned_by_namespace(observation_type, owning_namespace) {
            diagnostics.push(diag(
                format!("{path}.type"),
                "must be within owning_namespace",
            ));
        }
        if schemas_by_type
            .insert(observation_type.to_string(), path.clone())
            .is_some()
        {
            diagnostics.push(diag(
                format!("{path}.type"),
                "must have one deterministic schema registration",
            ));
        }
    }

    for (preset_index, preset) in document.policy.named_presets.iter().enumerate() {
        for (observation_index, observation) in preset.observations.iter().enumerate() {
            if !schemas_by_type.contains_key(observation.observation_type.as_str()) {
                diagnostics.push(diag(
                    format!("named_presets[{preset_index}].observations[{observation_index}].type"),
                    "must have a matching observation_schema_registration",
                ));
            }
        }
    }
}

fn validate_adapter_registrations(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    if document.policy.adapter_registrations.is_empty() {
        diagnostics.push(diag(
            "adapter_registrations",
            "must contain at least one registration",
        ));
        return;
    }
    let schema_types = registered_schema_types(document);

    for (adapter_index, adapter) in document.policy.adapter_registrations.iter().enumerate() {
        let path = format!("adapter_registrations[{adapter_index}]");
        for (type_index, observation_type) in string_array_at(adapter, &["observation_types"])
            .into_iter()
            .enumerate()
        {
            if !schema_types.contains(observation_type) {
                diagnostics.push(diag(
                    format!("{path}.observation_types[{type_index}]"),
                    "must have a matching observation schema registration",
                ));
            }
        }
        if !adapter.get("execution_contract").is_some_and(|execution| {
            string_at(execution, &["kind"]) == Some("process")
                && string_at(execution, &["executable"]).is_some_and(|value| !value.is_empty())
                && integer_at(execution, &["timeout_ms"]).is_some_and(|value| value > 0)
                && integer_at(execution, &["stdout_limit_bytes"]).is_some_and(|value| value > 0)
                && integer_at(execution, &["stderr_limit_bytes"]).is_some_and(|value| value > 0)
        }) {
            diagnostics.push(diag(
                format!("{path}.execution_contract"),
                "must declare a bounded process execution contract",
            ));
        }
    }
}

fn validate_policy_sections(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    if bool_at(
        &document.policy.layering_policy,
        &["weakening_requires_waiver"],
    ) != Some(true)
        || string_at(&document.policy.layering_policy, &["mode"]) != Some("monotonic_strengthening")
    {
        diagnostics.push(diag(
            "layering_policy",
            "must require monotonic strengthening and explicit waivers for weakening",
        ));
    }
    if string_at(&document.policy.fixture_policy, &["disclosure_required"]) == Some("false")
        || bool_at(&document.policy.fixture_policy, &["disclosure_required"]) != Some(true)
    {
        diagnostics.push(diag("fixture_policy.disclosure_required", "must be true"));
    }
    let layers = typed_frozen_policy_layers(document, diagnostics);
    validate_scope_chain(&layers, diagnostics);
}

fn validate_waivers(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    let observation_ids = document
        .policy
        .named_presets
        .iter()
        .flat_map(|preset| {
            preset
                .observations
                .iter()
                .map(|observation| observation.id.as_str())
        })
        .collect::<BTreeSet<_>>();
    let mut waiver_ids = BTreeSet::new();
    for (index, waiver) in document.waivers.iter().enumerate() {
        let path = format!("waivers[{index}]");
        if !waiver_ids.insert(waiver.id.as_str()) {
            diagnostics.push(diag(format!("{path}.id"), "must be unique"));
        }
        if waiver.reason.is_empty() {
            diagnostics.push(diag(format!("{path}.reason"), "must be non-empty"));
        }
        if waiver.created_by.is_empty() {
            diagnostics.push(diag(format!("{path}.created_by"), "must be non-empty"));
        }
        let created_at = parse_time(&waiver.created_at);
        let expires_at = parse_time(&waiver.expires_at);
        if created_at.is_none() {
            diagnostics.push(diag(format!("{path}.created_at"), "must be RFC3339"));
        }
        if expires_at.is_none() {
            diagnostics.push(diag(format!("{path}.expires_at"), "must be RFC3339"));
        }
        if let (Some(created_at), Some(expires_at)) = (created_at, expires_at)
            && expires_at <= created_at
        {
            diagnostics.push(diag(
                format!("{path}.expires_at"),
                "must be later than created_at",
            ));
        }
        for (observation_index, observation_id) in waiver.observation_ids.iter().enumerate() {
            if !observation_ids.contains(observation_id.as_str()) {
                diagnostics.push(diag(
                    format!("{path}.observation_ids[{observation_index}]"),
                    "must reference an observation produced by proof_presets",
                ));
            }
        }
        if !scope_has_explicit_id(waiver) {
            diagnostics.push(diag(
                format!("{path}.scope"),
                "must be explicitly scoped to goal, plan, item, or criterion",
            ));
        }
    }
}

fn registered_schema_types(document: &EvidencePolicyDocument) -> BTreeSet<&str> {
    let mut types = document
        .policy
        .observation_schema_registrations
        .iter()
        .filter_map(|registration| string_at(registration, &["type"]))
        .collect::<BTreeSet<_>>();
    types.extend([
        "planr.import.validator.generic_predicate",
        "planr.runner.result",
        "junit.xml.suite",
    ]);
    types
}

fn declared_extension_namespace(document: &EvidencePolicyDocument, namespace: &str) -> bool {
    document
        .policy
        .extension_namespaces
        .iter()
        .any(|declared| declared.as_str() == namespace)
}

fn is_reserved_namespace(namespace: &str) -> bool {
    RESERVED_EXTENSION_NAMESPACES
        .iter()
        .any(|root| namespace == *root || namespace.starts_with(&format!("{root}.")))
}

fn type_is_owned_by_namespace(observation_type: &str, namespace: &str) -> bool {
    observation_type == namespace
        || observation_type
            .strip_prefix(namespace)
            .is_some_and(|tail| tail.starts_with('.'))
}

fn scope_has_explicit_id(waiver: &EvidenceWaiver) -> bool {
    matches!(
        waiver.scope.kind.as_str(),
        "goal" | "plan" | "item" | "criterion"
    ) && !waiver.scope.id.as_str().is_empty()
}

#[derive(Debug, Clone)]
struct PolicyState {
    preset_id: String,
    binding: bool,
    fixtures_allowed: bool,
    mocks_allowed: bool,
    max_age_seconds: i64,
    assurance_level: String,
}

fn base_policy_state(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) -> PolicyState {
    let preset_id = string_at(&document.policy.defaults, &["preset_id"])
        .unwrap_or_default()
        .to_string();
    let binding = bool_at(&document.policy.defaults, &["binding"]).unwrap_or(true);
    let assurance_level = string_at(&document.policy.defaults, &["assurance_level"])
        .unwrap_or("standard")
        .to_string();
    let fixtures_allowed =
        bool_at(&document.policy.fixture_policy, &["fixtures_allowed"]).unwrap_or(false);
    let mocks_allowed =
        bool_at(&document.policy.fixture_policy, &["mocks_allowed"]).unwrap_or(false);
    let max_age_seconds =
        integer_at(&document.policy.freshness_policy, &["max_age_seconds"]).unwrap_or(1);
    if preset_id.is_empty() {
        diagnostics.push(diag("defaults.preset_id", "must be non-empty"));
    }
    PolicyState {
        preset_id,
        binding,
        fixtures_allowed,
        mocks_allowed,
        max_age_seconds,
        assurance_level,
    }
}

fn typed_frozen_policy_layers(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) -> Vec<FrozenPolicyLayer> {
    let Some(layers_value) = document
        .policy
        .layering_policy
        .get("layers")
        .and_then(Value::as_array)
    else {
        diagnostics.push(diag("layering_policy.layers", "must be an array"));
        return Vec::new();
    };
    let mut layers = Vec::new();
    for (index, value) in layers_value.iter().enumerate() {
        match serde_json::from_value::<FrozenPolicyLayer>(value.clone()) {
            Ok(layer) => {
                if let Err(error) = layer.scope.validate() {
                    diagnostics.push(diag(
                        format!("layering_policy.layers[{index}].scope"),
                        error.to_string(),
                    ));
                }
                layers.push(layer);
            }
            Err(error) => diagnostics.push(diag(
                format!("layering_policy.layers[{index}]"),
                format!("must match frozen PolicyLayer contract: {error}"),
            )),
        }
    }
    if layers.is_empty() {
        diagnostics.push(diag(
            "layering_policy.layers",
            "must contain at least one layer",
        ));
    }
    layers
}

fn validate_scope_chain(
    layers: &[FrozenPolicyLayer],
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    let mut previous_rank = None;
    let mut seen = BTreeSet::new();
    for layer in layers {
        let rank = scope_rank(&layer.scope.kind);
        if let Some(previous_rank) = previous_rank
            && rank < previous_rank
        {
            diagnostics.push(diag(
                "layering_policy.layers",
                "must be ordered goal -> plan -> item -> criterion",
            ));
        }
        previous_rank = Some(rank);
        if !seen.insert(scope_identity(&layer.scope)) {
            diagnostics.push(diag(
                format!(
                    "layering_policy.layers.{}:{}",
                    layer.scope.kind,
                    layer.scope.id.as_str()
                ),
                "must be unique in the scope chain",
            ));
        }
    }
}

fn validate_selected_scope_chain(
    layers: &[FrozenPolicyLayer],
    context: &EvidencePolicyResolutionContext,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    let mut matching_kinds = BTreeSet::new();
    for layer in layers {
        if layer_matches_selected_scope(layer, context)
            && !matching_kinds.insert(layer.scope.kind.clone())
        {
            diagnostics.push(diag(
                format!("layering_policy.layers.{}", layer.scope.kind),
                "must contain at most one layer for the selected scope kind",
            ));
        }
    }
}

fn layer_matches_selected_scope(
    layer: &FrozenPolicyLayer,
    context: &EvidencePolicyResolutionContext,
) -> bool {
    scope_matches_selected_chain(&layer.scope, &context.selected_scopes)
}

fn scope_identity(
    scope: &EvidenceScope,
) -> (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    (
        scope.kind.clone(),
        scope.id.as_str().to_string(),
        scope.plan_id.as_ref().map(|id| id.as_str().to_string()),
        scope.item_id.as_ref().map(|id| id.as_str().to_string()),
        scope
            .criterion_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
    )
}

fn typed_policy_materials(
    materials: &[EvidencePolicyLayerMaterial],
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) -> BTreeMap<String, TypedPolicyLayerMaterial> {
    let mut by_digest = BTreeMap::new();
    for (index, material) in materials.iter().enumerate() {
        let computed =
            sha256_json_digest_without_top_level_field(&material.value, "policy_digest").ok();
        match serde_json::from_value::<TypedPolicyLayerMaterial>(material.value.clone()) {
            Ok(typed) => {
                if computed.as_deref() != Some(typed.policy_digest.as_str()) {
                    diagnostics.push(diag(
                        format!("policy_materials[{index}].policy_digest"),
                        "must equal the canonical policy material digest",
                    ));
                    continue;
                }
                validate_policy_material(&typed, index, diagnostics);
                if by_digest
                    .insert(typed.policy_digest.as_str().to_string(), typed)
                    .is_some()
                {
                    diagnostics.push(diag(
                        format!("policy_materials[{index}].policy_digest"),
                        "must be unique",
                    ));
                }
            }
            Err(error) => diagnostics.push(diag(
                format!("policy_materials[{index}]"),
                format!("must match digest-addressed policy material contract: {error}"),
            )),
        }
    }
    by_digest
}

fn validate_policy_material(
    material: &TypedPolicyLayerMaterial,
    index: usize,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) {
    if material
        .max_age_seconds
        .is_some_and(|max_age_seconds| max_age_seconds <= 0)
    {
        diagnostics.push(diag(
            format!("policy_materials[{index}].max_age_seconds"),
            "must be positive",
        ));
    }
    if material
        .assurance_level
        .as_deref()
        .is_some_and(|assurance| assurance_rank(assurance).is_none())
    {
        diagnostics.push(diag(
            format!("policy_materials[{index}].assurance_level"),
            "must be low, standard, or high",
        ));
    }
}

fn preset_observations(
    document: &EvidencePolicyDocument,
    diagnostics: &mut Vec<EvidencePolicyDiagnostic>,
) -> BTreeMap<String, BTreeMap<String, Value>> {
    document
        .policy
        .named_presets
        .iter()
        .enumerate()
        .map(|(preset_index, preset)| {
            let observations = preset
                .observations
                .iter()
                .enumerate()
                .filter_map(|(observation_index, observation)| {
                    canonical_observation_value(observation)
                        .map(|value| (observation.id.as_str().to_string(), value))
                        .map_err(|error| {
                            diagnostics.push(diag(
                                format!(
                                    "named_presets[{preset_index}].observations[{observation_index}]"
                                ),
                                format!("must serialize canonically for policy comparison: {error}"),
                            ));
                        })
                        .ok()
                })
                .collect();
            (preset.id.as_str().to_string(), observations)
        })
        .collect()
}

fn canonical_observation_value(observation: &ObservationRequirement) -> anyhow::Result<Value> {
    serde_json::to_value(observation).map_err(Into::into)
}

fn observation_ids(observations: Option<&BTreeMap<String, Value>>) -> BTreeSet<String> {
    observations
        .iter()
        .flat_map(|values| values.keys().cloned())
        .collect()
}

fn weakened_observations(
    parent: &PolicyState,
    child: &PolicyState,
    preset_observations: &BTreeMap<String, BTreeMap<String, Value>>,
) -> BTreeSet<String> {
    let parent_observations = preset_observations.get(&parent.preset_id);
    let child_observations = preset_observations.get(&child.preset_id);
    let parent_ids = observation_ids(parent_observations);
    let child_ids = observation_ids(child_observations);
    parent_ids
        .union(&child_ids)
        .filter(|id| {
            let parent = parent_observations.and_then(|observations| observations.get(*id));
            let child = child_observations.and_then(|observations| observations.get(*id));
            parent.is_some() && parent != child
        })
        .cloned()
        .collect()
}

fn weakening_fields(
    parent: &PolicyState,
    child: &PolicyState,
    weakens_observations: bool,
) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if weakens_observations {
        fields.push("observations");
    }
    if parent.binding && !child.binding {
        fields.push("binding");
    }
    if !parent.fixtures_allowed && child.fixtures_allowed {
        fields.push("fixtures_allowed");
    }
    if !parent.mocks_allowed && child.mocks_allowed {
        fields.push("mocks_allowed");
    }
    if child.max_age_seconds > parent.max_age_seconds {
        fields.push("max_age_seconds");
    }
    if assurance_rank(&child.assurance_level).unwrap_or(-1)
        < assurance_rank(&parent.assurance_level).unwrap_or(-1)
    {
        fields.push("assurance_level");
    }
    fields
}

fn assurance_rank(value: &str) -> Option<i32> {
    Some(match value {
        "low" => 0,
        "standard" => 1,
        "high" => 2,
        _ => return None,
    })
}

fn scope_rank(scope_kind: &str) -> usize {
    match scope_kind {
        "goal" => 0,
        "plan" => 1,
        "item" => 2,
        "criterion" => 3,
        _ => 99,
    }
}

impl SelectedPolicyScopes {
    fn selected_id(&self, scope_kind: &str) -> Option<&str> {
        match scope_kind {
            "goal" => self.goal_id.as_deref(),
            "plan" => self.plan_id.as_deref(),
            "item" => self.item_id.as_deref(),
            "criterion" => self.criterion_id.as_deref(),
            _ => None,
        }
    }
}

fn scope_matches_selected_chain(scope: &EvidenceScope, selected: &SelectedPolicyScopes) -> bool {
    selected
        .selected_id(&scope.kind)
        .is_some_and(|selected_id| selected_id == scope.id.as_str())
        && optional_scope_binding_matches(scope.plan_id.as_ref(), selected.plan_id.as_deref())
        && optional_scope_binding_matches(scope.item_id.as_ref(), selected.item_id.as_deref())
        && optional_scope_binding_matches(
            scope.criterion_id.as_ref(),
            selected.criterion_id.as_deref(),
        )
}

fn optional_scope_binding_matches(binding: Option<&EvidenceId>, selected_id: Option<&str>) -> bool {
    binding.is_none_or(|binding| selected_id == Some(binding.as_str()))
}

fn scopes_are_exactly_equal(left: &EvidenceScope, right: &EvidenceScope) -> bool {
    left.kind == right.kind
        && left.id.as_str() == right.id.as_str()
        && optional_scope_ids_equal(left.plan_id.as_ref(), right.plan_id.as_ref())
        && optional_scope_ids_equal(left.item_id.as_ref(), right.item_id.as_ref())
        && optional_scope_ids_equal(left.criterion_id.as_ref(), right.criterion_id.as_ref())
}

fn optional_scope_ids_equal(left: Option<&EvidenceId>, right: Option<&EvidenceId>) -> bool {
    left.map(EvidenceId::as_str) == right.map(EvidenceId::as_str)
}

fn waiver_covers_layer(
    document: &EvidencePolicyDocument,
    layer: &FrozenPolicyLayer,
    context: &EvidencePolicyResolutionContext,
    weakened_observations: &BTreeSet<String>,
) -> bool {
    document.waivers.iter().any(|waiver| {
        scopes_are_exactly_equal(&waiver.scope, &layer.scope)
            && waiver_observations_cover(waiver, context, weakened_observations)
            && source_matches(waiver, context)
            && target_matches(waiver, context)
            && parse_time(&waiver.created_at)
                .zip(parse_time(&waiver.expires_at))
                .is_some_and(|(created_at, expires_at)| {
                    created_at <= context.evaluated_at && context.evaluated_at < expires_at
                })
    })
}

fn waiver_observations_cover(
    waiver: &EvidenceWaiver,
    context: &EvidencePolicyResolutionContext,
    weakened_observations: &BTreeSet<String>,
) -> bool {
    let covered = waiver
        .observation_ids
        .iter()
        .map(|id| id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let required = context
        .observation_ids
        .union(weakened_observations)
        .cloned()
        .collect::<BTreeSet<_>>();
    !covered.is_empty()
        && required
            .iter()
            .all(|observation| covered.contains(observation))
}

fn source_matches(waiver: &EvidenceWaiver, context: &EvidencePolicyResolutionContext) -> bool {
    waiver.source.revision == context.source.revision
        && waiver.source.tree_digest.as_str() == context.source.tree_digest.as_str()
        && waiver.source.dirty == context.source.dirty
}

fn target_matches(waiver: &EvidenceWaiver, context: &EvidencePolicyResolutionContext) -> bool {
    waiver.target.kind == context.target.kind
        && waiver.target.uri == context.target.uri
        && waiver.target.digest.as_ref().map(Sha256Digest::as_str)
            == context.target.digest.as_ref().map(Sha256Digest::as_str)
        && waiver.target.deployment_id.as_ref().map(EvidenceId::as_str)
            == context
                .target
                .deployment_id
                .as_ref()
                .map(EvidenceId::as_str)
}

fn parse_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_str()
}

fn string_array_at<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a str> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_bool()
}

fn integer_at(value: &Value, path: &[&str]) -> Option<i64> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_i64()
}

fn diagnostics(diagnostics: Vec<EvidencePolicyDiagnostic>) -> EvidencePolicyDiagnostics {
    EvidencePolicyDiagnostics { diagnostics }
}

fn diag(path: impl Into<String>, message: impl Into<String>) -> EvidencePolicyDiagnostic {
    EvidencePolicyDiagnostic {
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_json::sha256_json_digest;
    use crate::evidence::model::CapabilityAvailabilityStatus;
    use crate::evidence::registry::{
        CapabilityRegistry, CapabilityRegistryDiagnosticCode, CapabilityRuntimeContext,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    fn fixture_policy_yaml() -> String {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-policy.json"
        ))
        .unwrap();
        rewrite_policy_namespace(&mut value, "project.api");
        value["policy_digest"] = Value::String(policy_digest(&value));
        serde_yaml::to_string(&value).unwrap()
    }

    fn parsed_value() -> Value {
        serde_yaml::from_str::<Value>(&fixture_policy_yaml()).unwrap()
    }

    fn yaml_from_value(mut value: Value) -> String {
        value["policy_digest"] = Value::String(policy_digest(&value));
        serde_yaml::to_string(&value).unwrap()
    }

    fn policy_digest(value: &Value) -> String {
        sha256_json_digest(&policy_digest_preimage(value).unwrap()).unwrap()
    }

    fn layer_material(mut value: Value) -> EvidencePolicyLayerMaterial {
        let digest = sha256_json_digest_without_top_level_field(&value, "policy_digest").unwrap();
        value["policy_digest"] = Value::String(digest);
        EvidencePolicyLayerMaterial { value }
    }

    fn weakening_material() -> EvidencePolicyLayerMaterial {
        layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "binding": false,
            "fixtures_allowed": true,
            "max_age_seconds": 7200,
            "assurance_level": "low"
        }))
    }

    fn context_from_waiver(waiver: &Value, evaluated_at: &str) -> EvidencePolicyResolutionContext {
        EvidencePolicyResolutionContext {
            evaluated_at: parse_time(evaluated_at).unwrap(),
            source: serde_json::from_value(waiver["source"].clone()).unwrap(),
            target: serde_json::from_value(waiver["target"].clone()).unwrap(),
            observation_ids: BTreeSet::from(["obs-http-200".to_string()]),
            selected_scopes: SelectedPolicyScopes {
                plan_id: Some("pln-example".to_string()),
                item_id: Some("item-example".to_string()),
                criterion_id: Some("crit-1".to_string()),
                ..SelectedPolicyScopes::default()
            },
        }
    }

    fn context_with_scopes(scopes: &[(&str, &str)]) -> EvidencePolicyResolutionContext {
        let mut context = fixture_resolution_context();
        context.selected_scopes = SelectedPolicyScopes::default();
        for (kind, id) in scopes {
            match *kind {
                "goal" => context.selected_scopes.goal_id = Some(id.to_string()),
                "plan" => context.selected_scopes.plan_id = Some(id.to_string()),
                "item" => context.selected_scopes.item_id = Some(id.to_string()),
                "criterion" => context.selected_scopes.criterion_id = Some(id.to_string()),
                unknown => panic!("unknown test scope kind {unknown}"),
            }
        }
        context
    }

    fn fixture_resolution_context() -> EvidencePolicyResolutionContext {
        let waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        context_from_waiver(&waiver, "2026-07-28T12:30:00Z")
    }

    fn add_other_preset(value: &mut Value) {
        let mut other = value["named_presets"][0].clone();
        other["id"] = Value::String("preset-other".to_string());
        other["observations"][0]["id"] = Value::String("obs-other".to_string());
        value["named_presets"].as_array_mut().unwrap().push(other);
    }

    fn evidence_schema_validator() -> jsonschema::Validator {
        let schema: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json"
        ))
        .unwrap();
        jsonschema::draft202012::options().build(&schema).unwrap()
    }

    struct DisposableQueueFixture {
        repo: TempDir,
        document: EvidencePolicyDocument,
        expanded: Vec<ExpandedProofObservation>,
        schema_path: PathBuf,
    }

    fn disposable_queue_fixture() -> DisposableQueueFixture {
        let repo = tempdir().unwrap();
        let schema_path = repo
            .path()
            .join(".planr/evidence/schemas/example.queue.job.processed.v1.schema.json");
        let manifest_path = repo
            .path()
            .join(".planr/evidence/adapters/queue-worker.manifest.json");
        fs::create_dir_all(schema_path.parent().unwrap()).unwrap();
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "example.queue.job.processed@v1",
            "type": "object",
            "required": ["job_id", "status"],
            "properties": {
                "job_id": {"type": "string"},
                "status": {"const": "processed"}
            },
            "additionalProperties": false
        });
        fs::write(&schema_path, serde_json::to_vec_pretty(&schema).unwrap()).unwrap();
        let schema_digest = sha256_json_digest(&schema).unwrap();
        let payload_schema = json!({
            "type": "example.queue.job.processed",
            "schema_ref": "example.queue.job.processed@v1",
            "schema_digest": schema_digest
        });
        let execution_contract = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", "printf queue-worker-ready"],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 4096,
            "stderr_limit_bytes": 4096,
            "payload_schema": payload_schema
        });
        let manifest = json!({
            "id": "vcap-example-queue-worker-v1",
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "process",
            "adapter_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "supported_surfaces": ["local-process"],
            "supported_observations": [payload_schema],
            "supported_interactions": ["process"],
            "supported_artifacts": ["stdout"],
            "runtime_targets": [{"kind": "process", "id": "queue-worker"}],
            "provenance_path": "planr_observed_execution",
            "permissions": {"network": "none", "filesystem": "read_workspace"},
            "costs": {},
            "determinism": "deterministic",
            "repeatability": "repeatable in a disposable repository",
            "independence": "repository-owned adapter manifest outside Planr core",
            "blind_spots": ["fixture only proves registration and discovery"],
            "availability_probe": {
                "kind": "process",
                "execution": execution_contract
            }
        });
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let manifest_digest = sha256_json_digest(&manifest).unwrap();

        let policy = json!({
            "id": "epolicy-example-queue-v1",
            "schema_version": "evidence.contract.v1",
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "defaults": {
                "preset_id": "queue-worker",
                "binding": true,
                "assurance_level": "standard"
            },
            "named_presets": [{
                "id": "queue-worker",
                "schema_version": "evidence.contract.v1",
                "namespace": "example.queue.job",
                "observations": [{
                    "id": "job-processed",
                    "type": "example.queue.job.processed",
                    "subject": "queue job processed",
                    "expected": {"status": "processed"},
                    "target": {"kind": "queue", "id": "jobs"}
                }]
            }],
            "observation_schema_registrations": [{
                "type": "example.queue.job.processed",
                "schema_ref": "example.queue.job.processed@v1",
                "schema_digest": schema_digest,
                "owning_namespace": "example.queue.job"
            }],
            "adapter_registrations": [{
                "manifest_id": "vcap-example-queue-worker-v1",
                "manifest_path": ".planr/evidence/adapters/queue-worker.manifest.json",
                "manifest_digest": manifest_digest,
                "observation_types": ["example.queue.job.processed"],
                "payload_schemas": [payload_schema],
                "provenance_path": "planr_observed_execution",
                "execution_contract": execution_contract
            }],
            "extension_namespaces": ["example.queue.job"],
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
                "fixtures_allowed": true,
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
                    "scope": {"kind": "plan", "id": "pln-example"},
                    "policy_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }]
            }
        });
        let policy_path = repo.path().join(".planr/evidence.yaml");
        fs::write(&policy_path, yaml_from_value(policy)).unwrap();

        let policy_yaml = fs::read_to_string(&policy_path).unwrap();
        let document = parse_evidence_policy_yaml(&policy_yaml).unwrap();
        let expanded = expand_proof_presets(&document).unwrap();
        DisposableQueueFixture {
            repo,
            document,
            expanded,
            schema_path,
        }
    }

    fn capability_conn() -> Connection {
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

    fn queue_runtime() -> CapabilityRuntimeContext<'static> {
        CapabilityRuntimeContext {
            host: "codex",
            surface: "local-process",
            host_version: "test",
            environment_id: "disposable-repo",
        }
    }

    fn rewrite_policy_namespace(value: &mut Value, namespace: &str) {
        value["extension_namespaces"] = Value::Array(vec![Value::String(namespace.to_string())]);
        value["named_presets"][0]["namespace"] = Value::String(namespace.to_string());
        value["named_presets"][0]["observations"][0]["type"] =
            Value::String(format!("{namespace}.http.response"));
        value["observation_schema_registrations"][0]["type"] =
            Value::String(format!("{namespace}.http.response"));
        value["observation_schema_registrations"][0]["owning_namespace"] =
            Value::String(namespace.to_string());
        value["adapter_registrations"][0]["observation_types"] =
            Value::Array(vec![Value::String(format!("{namespace}.http.response"))]);
        value["adapter_registrations"][0]["payload_schemas"][0]["type"] =
            Value::String(format!("{namespace}.http.response"));
        value["adapter_registrations"][0]["execution_contract"]["payload_schema"]["type"] =
            Value::String(format!("{namespace}.http.response"));
    }

    fn expect_error(mut value: Value, needle: &str) {
        let yaml = yaml_from_value(value.take());
        let error = parse_evidence_policy_yaml(&yaml).unwrap_err().to_string();
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn evidence_policy_yaml_fixture_parses_and_validates_digest() {
        let document = parse_evidence_policy_yaml(&fixture_policy_yaml()).unwrap();
        assert_eq!(document.policy.id.as_str(), "epolicy-default-v1");
        assert_eq!(document.policy.policy_digest.as_str(), document.digest);
        assert_eq!(document.policy.named_presets.len(), 1);
    }

    #[test]
    fn disposable_repository_fixture_registers_custom_queue_worker_capability() {
        let fixture = disposable_queue_fixture();
        validate_repository_observation_schemas(fixture.repo.path(), &fixture.document).unwrap();
        assert_eq!(
            fixture.document.policy.extension_namespaces[0].as_str(),
            "example.queue.job"
        );

        assert_eq!(fixture.expanded.len(), 1);
        assert_eq!(fixture.expanded[0].preset_id, "queue-worker");
        assert_eq!(
            fixture.expanded[0].observation_type,
            "example.queue.job.processed"
        );

        let mut registry = CapabilityRegistry::from_manifests_and_adapter_registrations(
            fixture.repo.path(),
            [],
            &fixture.document.policy.adapter_registrations,
        );
        assert_eq!(registry.diagnostics(), []);
        let capabilities = registry.capabilities().collect::<Vec<_>>();
        assert_eq!(capabilities.len(), 1);
        assert_eq!(
            capabilities[0].manifest.id.as_str(),
            "vcap-example-queue-worker-v1"
        );
        assert!(
            capabilities[0]
                .manifest
                .supported_observations
                .iter()
                .any(|binding| binding.observation_type.as_str() == "example.queue.job.processed")
        );

        let diagnostics_before_probe = registry.available_diagnostics_for_declared_observations(
            fixture
                .expanded
                .iter()
                .map(|observation| observation.observation_type.clone()),
        );
        assert_eq!(diagnostics_before_probe.len(), 1);
        assert_eq!(
            diagnostics_before_probe[0].code,
            CapabilityRegistryDiagnosticCode::DeclaredManifestUnavailable
        );
        assert!(
            diagnostics_before_probe[0]
                .message
                .contains("registered capabilities")
        );

        let conn = capability_conn();
        let instance = registry
            .probe_and_store(
                &conn,
                fixture.repo.path(),
                "vcap-example-queue-worker-v1",
                queue_runtime(),
            )
            .unwrap();
        assert_eq!(
            instance.manifest_id.as_str(),
            "vcap-example-queue-worker-v1"
        );
        assert_eq!(
            instance.availability.status,
            CapabilityAvailabilityStatus::Available
        );
        assert_eq!(
            instance.observed_payload_contract.schema_ref,
            "example.queue.job.processed@v1"
        );
        assert!(
            instance
                .observed_payload_contract
                .observation_types
                .iter()
                .any(|observation_type| observation_type.as_str() == "example.queue.job.processed")
        );
        let stored_instances: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verification_capability_instances WHERE manifest_id = 'vcap-example-queue-worker-v1' AND availability_status = 'available'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_instances, 1);
        assert!(
            registry
                .available_diagnostics_for_declared_observations(
                    fixture
                        .expanded
                        .iter()
                        .map(|observation| observation.observation_type.clone()),
                )
                .is_empty()
        );
    }

    #[test]
    fn repository_schema_validation_rejects_missing_or_tampered_queue_schema() {
        let fixture = disposable_queue_fixture();
        fs::remove_file(&fixture.schema_path).unwrap();
        let error = validate_repository_observation_schemas(fixture.repo.path(), &fixture.document)
            .unwrap_err()
            .to_string();
        assert!(error.contains("must be readable"), "{error}");

        let fixture = disposable_queue_fixture();
        fs::write(
            &fixture.schema_path,
            serde_json::to_vec_pretty(&json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "example.queue.job.processed@v1",
                "type": "object",
                "additionalProperties": true
            }))
            .unwrap(),
        )
        .unwrap();
        let error = validate_repository_observation_schemas(fixture.repo.path(), &fixture.document)
            .unwrap_err()
            .to_string();
        assert!(error.contains("schema_digest"), "{error}");
    }

    #[test]
    fn versioned_schema_ref_rejects_at_sign_filename_and_reports_canonical_path() {
        let fixture = disposable_queue_fixture();
        let noncanonical = fixture
            .schema_path
            .with_file_name("example.queue.job.processed@v1.schema.json");
        fs::rename(&fixture.schema_path, &noncanonical).unwrap();

        let error = load_repository_observation_schema(
            fixture.repo.path(),
            "example.queue.job.processed@v1",
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("example.queue.job.processed.v1.schema.json must be readable"),
            "{error}"
        );
    }

    #[test]
    fn trusted_builtin_policy_yaml_can_use_reserved_planr_namespace() {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-policy.json"
        ))
        .unwrap();
        value["policy_digest"] = Value::String(policy_digest(&value));
        let document =
            parse_trusted_builtin_evidence_policy_yaml(&serde_yaml::to_string(&value).unwrap())
                .unwrap();
        assert_eq!(
            document.policy.extension_namespaces[0].as_str(),
            "planr.api"
        );
    }

    #[test]
    fn evidence_policy_yaml_rejects_digest_drift_and_unknown_fields() {
        let mut value = parsed_value();
        value["policy_digest"] = Value::String(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        );
        let yaml = serde_yaml::to_string(&value).unwrap();
        let error = parse_evidence_policy_yaml(&yaml).unwrap_err().to_string();
        assert!(error.contains("policy_digest"), "{error}");

        let mut value = parsed_value();
        value["unexpected"] = Value::Bool(true);
        let yaml = yaml_from_value(value);
        let error = parse_evidence_policy_yaml(&yaml).unwrap_err().to_string();
        assert!(
            error.contains("unknown field") || error.contains("additional"),
            "{error}"
        );
    }

    #[test]
    fn evidence_policy_yaml_rejects_unregistered_presets_and_custom_schemas() {
        let mut value = parsed_value();
        value["defaults"]["preset_id"] = Value::String("missing-preset".to_string());
        expect_error(value, "defaults.preset_id");

        let mut value = parsed_value();
        value["named_presets"][0]["observations"][0]["type"] =
            Value::String("project.queue.job.processed".to_string());
        expect_error(value, "matching observation_schema_registration");

        let mut value = parsed_value();
        value["observation_schema_registrations"][0]["owning_namespace"] =
            Value::String("planr".to_string());
        expect_error(value, "reserved namespace");
    }

    #[test]
    fn evidence_policy_yaml_rejects_reserved_namespace_descendants() {
        for namespace in ["planr.custom", "mcp.custom", "host.custom"] {
            let mut value = parsed_value();
            rewrite_policy_namespace(&mut value, namespace);
            expect_error(value, "reserved namespace");
        }
    }

    #[test]
    fn evidence_policy_yaml_enforces_monotonic_layering_and_adapter_bounds() {
        let mut value = parsed_value();
        value["layering_policy"]["weakening_requires_waiver"] = Value::Bool(false);
        expect_error(value, "layering_policy");

        let mut value = parsed_value();
        value["adapter_registrations"][0]["execution_contract"]["timeout_ms"] = Value::from(0);
        expect_error(value, "bounded process execution contract");
    }

    #[test]
    fn evidence_policy_yaml_accepts_explicit_expiring_waivers() {
        let mut value = parsed_value();
        let waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        value["waivers"] = Value::Array(vec![waiver]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        assert_eq!(document.waivers.len(), 1);
    }

    #[test]
    fn evidence_policy_yaml_rejects_unscoped_or_stale_waivers() {
        let mut value = parsed_value();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["scope"]["kind"] = Value::String("project".to_string());
        value["waivers"] = Value::Array(vec![waiver]);
        expect_error(value, "scope.kind");

        let mut value = parsed_value();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["expires_at"] = waiver["created_at"].clone();
        value["waivers"] = Value::Array(vec![waiver]);
        expect_error(value, "later than created_at");
    }

    #[test]
    fn evidence_policy_yaml_rejects_incomplete_waiver_contract_fields() {
        for (field, needle) in [
            ("observation_ids", "observation_ids"),
            ("source", "source"),
            ("target", "target"),
            ("approval_ref", "approval_ref"),
            ("audit_trail", "audit_trail"),
        ] {
            let mut value = parsed_value();
            let mut waiver: Value = serde_json::from_str(include_str!(
                "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
            ))
            .unwrap();
            waiver.as_object_mut().unwrap().remove(field);
            value["waivers"] = Value::Array(vec![waiver]);
            expect_error(value, needle);
        }
    }

    #[test]
    fn evidence_policy_yaml_compares_waiver_expiry_as_instant() {
        let mut value = parsed_value();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["created_at"] = Value::String("2026-07-28T12:00:00+02:00".to_string());
        waiver["expires_at"] = Value::String("2026-07-28T10:30:00Z".to_string());
        value["waivers"] = Value::Array(vec![waiver]);
        parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();

        let mut value = parsed_value();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["created_at"] = Value::String("2026-07-28T10:30:00Z".to_string());
        waiver["expires_at"] = Value::String("2026-07-28T12:00:00+02:00".to_string());
        value["waivers"] = Value::Array(vec![waiver]);
        expect_error(value, "later than created_at");
    }

    #[test]
    fn proof_preset_expansion_is_authoring_order_stable_with_derived_ids() {
        let mut value = parsed_value();
        let mut second = value["named_presets"][0].clone();
        second["id"] = Value::String("preset-z".to_string());
        second["observations"][0]["id"] = Value::String("obs-z".to_string());
        value["named_presets"].as_array_mut().unwrap().push(second);
        let first = parse_evidence_policy_yaml(&yaml_from_value(value.clone())).unwrap();

        value["named_presets"].as_array_mut().unwrap().reverse();
        let second = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let first_expanded = expand_proof_presets(&first).unwrap();
        let second_expanded = expand_proof_presets(&second).unwrap();
        assert_eq!(first_expanded, second_expanded);
        assert!(
            first_expanded
                .iter()
                .any(|expanded| { expanded.derived_id == "preset-http-health:obs-http-200" })
        );
    }

    #[test]
    fn policy_layers_resolve_in_scope_order_and_accept_strengthening() {
        let mut value = parsed_value();
        let plan_policy = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "fixtures_allowed": false,
            "mocks_allowed": false,
            "max_age_seconds": 300,
            "assurance_level": "standard"
        }));
        let criterion_policy = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 60,
            "assurance_level": "high"
        }));
        value["layering_policy"]["layers"] = serde_json::json!([
            {
                "scope": {"kind": "plan", "id": "pln-1"},
                "policy_digest": plan_policy.value["policy_digest"]
            },
            {
                "scope": {"kind": "criterion", "id": "crit-1"},
                "policy_digest": criterion_policy.value["policy_digest"]
            }
        ]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let resolved = resolve_policy_layers(
            &document,
            &[plan_policy, criterion_policy],
            &context_with_scopes(&[("plan", "pln-1"), ("criterion", "crit-1")]),
        )
        .unwrap();
        assert_eq!(resolved[0].scope_kind, "plan");
        assert_eq!(resolved[1].scope_kind, "criterion");
        assert_eq!(resolved[1].max_age_seconds, 60);
        assert_eq!(resolved[1].assurance_level, "high");
    }

    #[test]
    fn policy_layers_reject_unwaived_weakening_and_accept_matching_waiver() {
        let mut value = parsed_value();
        let weakening = weakening_material();
        value["layering_policy"]["layers"][0]["policy_digest"] =
            weakening.value["policy_digest"].clone();
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let error = resolve_policy_layers(
            &document,
            std::slice::from_ref(&weakening),
            &fixture_resolution_context(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("weakening requires matching waiver"),
            "{error}"
        );

        let mut value = parsed_value();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["id"] = Value::String("waiver-layer".to_string());
        waiver["scope"]["kind"] = Value::String("plan".to_string());
        waiver["scope"]["id"] = Value::String("pln-example".to_string());
        waiver["scope"].as_object_mut().unwrap().remove("plan_id");
        waiver["scope"].as_object_mut().unwrap().remove("item_id");
        waiver["scope"]
            .as_object_mut()
            .unwrap()
            .remove("criterion_id");
        let weakening = weakening_material();
        value["layering_policy"]["layers"][0]["policy_digest"] =
            weakening.value["policy_digest"].clone();
        value["waivers"] = Value::Array(vec![waiver]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let resolved =
            resolve_policy_layers(&document, &[weakening], &fixture_resolution_context()).unwrap();
        assert!(!resolved[0].binding);
        assert!(resolved[0].fixtures_allowed);
    }

    #[test]
    fn policy_layers_reject_unknown_digest_unknown_preset_and_weaker_preset() {
        let mut value = parsed_value();
        let document = parse_evidence_policy_yaml(&yaml_from_value(value.clone())).unwrap();
        let error = resolve_policy_layers(&document, &[], &fixture_resolution_context())
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing policy material"), "{error}");

        let unknown_preset = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "preset_id": "preset-missing"
        }));
        value["layering_policy"]["layers"][0]["policy_digest"] =
            unknown_preset.value["policy_digest"].clone();
        let document = parse_evidence_policy_yaml(&yaml_from_value(value.clone())).unwrap();
        let error = resolve_policy_layers(
            &document,
            std::slice::from_ref(&unknown_preset),
            &fixture_resolution_context(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("declared proof preset"), "{error}");

        let mut weaker_preset_value = value;
        add_other_preset(&mut weaker_preset_value);
        let weaker_preset = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "preset_id": "preset-other"
        }));
        weaker_preset_value["layering_policy"]["layers"][0]["policy_digest"] =
            weaker_preset.value["policy_digest"].clone();
        let document = parse_evidence_policy_yaml(&yaml_from_value(weaker_preset_value)).unwrap();
        let error =
            resolve_policy_layers(&document, &[weaker_preset], &fixture_resolution_context())
                .unwrap_err()
                .to_string();
        assert!(error.contains("observations"), "{error}");
    }

    #[test]
    fn policy_layers_keep_frozen_schema_shape_and_parser_compatibility() {
        let mut value = parsed_value();
        let material = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 300
        }));
        value["layering_policy"]["layers"][0]["policy_digest"] =
            material.value["policy_digest"].clone();
        let yaml = yaml_from_value(value.clone());
        let validator = evidence_schema_validator();
        let errors = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "{errors:?}");
        parse_evidence_policy_yaml(&yaml).unwrap();

        value["layering_policy"]["layers"][0]["waiver_id"] =
            Value::String("waiver-layer".to_string());
        let yaml = yaml_from_value(value);
        let error = parse_evidence_policy_yaml(&yaml).unwrap_err().to_string();
        assert!(error.contains("frozen PolicyLayer contract"), "{error}");
    }

    #[test]
    fn policy_layers_scope_contract_matches_schema_corpus() {
        let validator = evidence_schema_validator();
        for (kind, optional_fields) in [
            ("goal", serde_json::json!({})),
            ("plan", serde_json::json!({"plan_id": "pln-example"})),
            (
                "item",
                serde_json::json!({"plan_id": "pln-example", "item_id": "item-example"}),
            ),
            (
                "criterion",
                serde_json::json!({
                    "plan_id": "pln-example",
                    "item_id": "item-example",
                    "criterion_id": "crit-example"
                }),
            ),
        ] {
            let mut value = parsed_value();
            let mut scope = serde_json::json!({"kind": kind, "id": format!("{kind}-example")});
            scope.as_object_mut().unwrap().extend(
                optional_fields
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
            value["layering_policy"]["layers"][0]["scope"] = scope;
            let errors = validator
                .iter_errors(&value)
                .map(|error| error.to_string())
                .collect::<Vec<_>>();
            assert!(errors.is_empty(), "{kind}: {errors:?}");
            parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        }

        for kind in ["repository", "product"] {
            let mut value = parsed_value();
            value["layering_policy"]["layers"][0]["scope"] =
                serde_json::json!({"kind": kind, "id": format!("{kind}-example")});
            let schema_errors = validator
                .iter_errors(&value)
                .map(|error| error.to_string())
                .collect::<Vec<_>>();
            assert!(!schema_errors.is_empty(), "{kind} should fail schema");
            let error = parse_evidence_policy_yaml(&yaml_from_value(value))
                .unwrap_err()
                .to_string();
            assert!(error.contains("scope.kind"), "{error}");
        }
    }

    #[test]
    fn evidence_contract_documents_repository_product_to_frozen_scope_mapping() {
        let contract = include_str!("../../docs/contracts/EVIDENCE_CONTRACT_V1.md");
        assert!(contract.contains(
            "Repository policy defaults and extension declarations are document-level policy"
        ));
        assert!(contract.contains("Product-level policy is represented by `goal`"));
        assert!(contract.contains("plan-level policy by `plan`"));
        assert!(contract.contains("criterion-level policy by `criterion`"));
    }

    #[test]
    fn policy_layers_treat_same_id_weaker_observation_replacement_as_weakening() {
        let mut value = parsed_value();
        let mut weaker = value["named_presets"][0].clone();
        weaker["id"] = Value::String("preset-weaker".to_string());
        weaker["observations"][0]["expected"]["status"] = Value::from(204);
        value["named_presets"].as_array_mut().unwrap().push(weaker);
        let material = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "preset_id": "preset-weaker"
        }));
        value["layering_policy"]["layers"][0]["policy_digest"] =
            material.value["policy_digest"].clone();
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let error = resolve_policy_layers(&document, &[material], &fixture_resolution_context())
            .unwrap_err()
            .to_string();
        assert!(error.contains("observations"), "{error}");
    }

    #[test]
    fn policy_layers_do_not_cross_apply_unrelated_scope_branches() {
        let mut value = parsed_value();
        let plan_a = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 300
        }));
        let plan_b = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 30
        }));
        let criterion_a = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "assurance_level": "high"
        }));
        let criterion_b = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 10,
            "assurance_level": "high"
        }));
        value["layering_policy"]["layers"] = serde_json::json!([
            {
                "scope": {"kind": "plan", "id": "plan-a"},
                "policy_digest": plan_a.value["policy_digest"]
            },
            {
                "scope": {"kind": "plan", "id": "plan-b"},
                "policy_digest": plan_b.value["policy_digest"]
            },
            {
                "scope": {"kind": "criterion", "id": "criterion-a"},
                "policy_digest": criterion_a.value["policy_digest"]
            },
            {
                "scope": {"kind": "criterion", "id": "criterion-b"},
                "policy_digest": criterion_b.value["policy_digest"]
            }
        ]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let resolved = resolve_policy_layers(
            &document,
            &[plan_a, plan_b, criterion_a, criterion_b],
            &context_with_scopes(&[("plan", "plan-a"), ("criterion", "criterion-a")]),
        )
        .unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].scope_id, "plan-a");
        assert_eq!(resolved[0].max_age_seconds, 300);
        assert_eq!(resolved[1].scope_id, "criterion-a");
        assert_eq!(resolved[1].max_age_seconds, 300);
        assert_eq!(resolved[1].assurance_level, "high");
    }

    #[test]
    fn policy_layers_do_not_cross_apply_same_ids_under_different_parents() {
        let mut value = parsed_value();
        let plan_policy = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 300
        }));
        let item_a = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 120
        }));
        let item_b = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 60
        }));
        let criterion_a = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "assurance_level": "high"
        }));
        let criterion_b = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 30,
            "assurance_level": "high"
        }));
        value["layering_policy"]["layers"] = serde_json::json!([
            {
                "scope": {"kind": "plan", "id": "plan-a"},
                "policy_digest": plan_policy.value["policy_digest"]
            },
            {
                "scope": {"kind": "item", "id": "shared-item", "plan_id": "plan-a"},
                "policy_digest": item_a.value["policy_digest"]
            },
            {
                "scope": {"kind": "item", "id": "shared-item", "plan_id": "plan-b"},
                "policy_digest": item_b.value["policy_digest"]
            },
            {
                "scope": {
                    "kind": "criterion",
                    "id": "shared-criterion",
                    "plan_id": "plan-a",
                    "item_id": "shared-item"
                },
                "policy_digest": criterion_a.value["policy_digest"]
            },
            {
                "scope": {
                    "kind": "criterion",
                    "id": "shared-criterion",
                    "plan_id": "plan-b",
                    "item_id": "shared-item"
                },
                "policy_digest": criterion_b.value["policy_digest"]
            }
        ]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let resolved = resolve_policy_layers(
            &document,
            &[plan_policy, item_a, item_b, criterion_a, criterion_b],
            &context_with_scopes(&[
                ("plan", "plan-a"),
                ("item", "shared-item"),
                ("criterion", "shared-criterion"),
            ]),
        )
        .unwrap();
        assert_eq!(
            resolved
                .iter()
                .map(|layer| layer.scope_kind.as_str())
                .collect::<Vec<_>>(),
            vec!["plan", "item", "criterion"]
        );
        assert_eq!(resolved[1].scope_id, "shared-item");
        assert_eq!(resolved[1].max_age_seconds, 120);
        assert_eq!(resolved[2].scope_id, "shared-criterion");
        assert_eq!(resolved[2].max_age_seconds, 120);
        assert_eq!(resolved[2].assurance_level, "high");
    }

    #[test]
    fn policy_layers_plan_only_context_skips_item_and_criterion_branches() {
        let mut value = parsed_value();
        let plan_policy = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 300
        }));
        let item_policy = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 60,
            "assurance_level": "high"
        }));
        let criterion_policy = layer_material(serde_json::json!({
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "max_age_seconds": 10,
            "assurance_level": "high"
        }));
        value["layering_policy"]["layers"] = serde_json::json!([
            {
                "scope": {"kind": "plan", "id": "pln-example"},
                "policy_digest": plan_policy.value["policy_digest"]
            },
            {
                "scope": {"kind": "item", "id": "item-specific", "plan_id": "pln-example"},
                "policy_digest": item_policy.value["policy_digest"]
            },
            {
                "scope": {
                    "kind": "criterion",
                    "id": "crit-specific",
                    "plan_id": "pln-example",
                    "item_id": "item-specific"
                },
                "policy_digest": criterion_policy.value["policy_digest"]
            }
        ]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let resolved = resolve_policy_layers(
            &document,
            &[plan_policy, item_policy, criterion_policy],
            &context_with_scopes(&[("plan", "pln-example")]),
        )
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].scope_kind, "plan");
        assert_eq!(resolved[0].max_age_seconds, 300);
        assert_eq!(resolved[0].assurance_level, "standard");
    }

    #[test]
    fn policy_layer_material_rejects_invalid_freshness_and_assurance_even_with_waiver() {
        for (field, material_value, needle) in [
            (
                "max_age_seconds",
                serde_json::json!({
                    "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "max_age_seconds": 0
                }),
                "max_age_seconds",
            ),
            (
                "max_age_seconds",
                serde_json::json!({
                    "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "max_age_seconds": -1
                }),
                "max_age_seconds",
            ),
            (
                "assurance_level",
                serde_json::json!({
                    "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "assurance_level": "medium"
                }),
                "assurance_level",
            ),
        ] {
            let mut value = parsed_value();
            let material = layer_material(material_value);
            value["layering_policy"]["layers"][0]["policy_digest"] =
                material.value["policy_digest"].clone();
            let mut waiver: Value = serde_json::from_str(include_str!(
                "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
            ))
            .unwrap();
            waiver["scope"]["kind"] = Value::String("plan".to_string());
            waiver["scope"]["id"] = Value::String("pln-example".to_string());
            value["waivers"] = Value::Array(vec![waiver]);
            let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
            let error =
                resolve_policy_layers(&document, &[material], &fixture_resolution_context())
                    .unwrap_err()
                    .to_string();
            assert!(error.contains(needle), "{field}: {error}");
        }
    }

    #[test]
    fn policy_layer_waivers_require_time_observation_and_binding_match() {
        let mut value = parsed_value();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["id"] = Value::String("waiver-layer".to_string());
        waiver["scope"]["kind"] = Value::String("plan".to_string());
        waiver["scope"]["id"] = Value::String("pln-example".to_string());
        let weakening = weakening_material();
        value["layering_policy"]["layers"][0]["policy_digest"] =
            weakening.value["policy_digest"].clone();

        let mut expired = value.clone();
        expired["waivers"] = Value::Array(vec![waiver.clone()]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(expired)).unwrap();
        let error = resolve_policy_layers(
            &document,
            std::slice::from_ref(&weakening),
            &context_from_waiver(&waiver, "2026-07-29T12:00:00Z"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("weakening requires matching waiver"),
            "{error}"
        );

        let mut unrelated_observation = waiver.clone();
        unrelated_observation["observation_ids"] =
            Value::Array(vec![Value::String("obs-other".to_string())]);
        let mut unrelated = value.clone();
        add_other_preset(&mut unrelated);
        unrelated["waivers"] = Value::Array(vec![unrelated_observation]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(unrelated)).unwrap();
        let error = resolve_policy_layers(
            &document,
            std::slice::from_ref(&weakening),
            &context_from_waiver(&waiver, "2026-07-28T12:30:00Z"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("weakening requires matching waiver"),
            "{error}"
        );

        let mut mismatched_binding = value;
        let mut mismatched_waiver = waiver.clone();
        mismatched_waiver["source"]["revision"] =
            Value::String("abcdef0123456789abcdef0123456789abcdef01".to_string());
        mismatched_binding["waivers"] = Value::Array(vec![mismatched_waiver]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(mismatched_binding)).unwrap();
        let error = resolve_policy_layers(
            &document,
            &[weakening],
            &context_from_waiver(&waiver, "2026-07-28T12:30:00Z"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("weakening requires matching waiver"),
            "{error}"
        );
    }

    #[test]
    fn policy_layer_waivers_require_exact_hierarchical_scope_match() {
        let mut value = parsed_value();
        let weakening = weakening_material();
        value["layering_policy"]["layers"][0]["scope"] = serde_json::json!({
            "kind": "item",
            "id": "shared-item",
            "plan_id": "plan-a"
        });
        value["layering_policy"]["layers"][0]["policy_digest"] =
            weakening.value["policy_digest"].clone();
        let mut waiver: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-waiver.json"
        ))
        .unwrap();
        waiver["scope"] = serde_json::json!({
            "kind": "item",
            "id": "shared-item",
            "plan_id": "plan-b"
        });
        value["waivers"] = Value::Array(vec![waiver.clone()]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value.clone())).unwrap();
        let error = resolve_policy_layers(
            &document,
            std::slice::from_ref(&weakening),
            &context_with_scopes(&[("plan", "plan-a"), ("item", "shared-item")]),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("weakening requires matching waiver"),
            "{error}"
        );

        waiver["scope"] = serde_json::json!({
            "kind": "item",
            "id": "shared-item",
            "plan_id": "plan-a"
        });
        value["waivers"] = Value::Array(vec![waiver]);
        let document = parse_evidence_policy_yaml(&yaml_from_value(value)).unwrap();
        let resolved = resolve_policy_layers(
            &document,
            &[weakening],
            &context_with_scopes(&[("plan", "plan-a"), ("item", "shared-item")]),
        )
        .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].scope_kind, "item");
        assert!(resolved[0].fixtures_allowed);
    }

    #[test]
    fn source_snapshot_excludes_mutable_planr_runtime_but_keeps_contract_and_product_files() {
        let repo = tempdir().unwrap();
        fs::create_dir_all(repo.path().join(".planr/evidence/runs")).unwrap();
        fs::create_dir_all(repo.path().join(".planr/evidence/adapters")).unwrap();
        fs::write(repo.path().join("product.txt"), "product-v1\n").unwrap();
        fs::write(repo.path().join(".planr/planr.sqlite"), "runtime-v1\n").unwrap();
        fs::write(
            repo.path().join(".planr/evidence/runs/run.json"),
            "{\"run\":1}\n",
        )
        .unwrap();
        fs::write(
            repo.path().join(".planr/evidence/adapters/adapter.mjs"),
            "process.stdout.write('{}')\n",
        )
        .unwrap();
        for args in [
            vec!["init"],
            vec!["config", "user.email", "planr-test@example.invalid"],
            vec!["config", "user.name", "Planr Test"],
            vec!["add", "."],
            vec!["commit", "-m", "initial"],
        ] {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "git failed: {output:?}");
        }

        let baseline = capture_source_binding(repo.path()).unwrap();
        fs::write(repo.path().join(".planr/planr.sqlite"), "runtime-v2\n").unwrap();
        fs::write(
            repo.path().join(".planr/evidence/runs/run.json"),
            "{\"run\":2}\n",
        )
        .unwrap();
        let runtime_changed = capture_source_binding(repo.path()).unwrap();
        assert_eq!(runtime_changed.tree_digest, baseline.tree_digest);
        assert!(!runtime_changed.dirty);

        fs::write(repo.path().join("product.txt"), "product-v2\n").unwrap();
        let product_changed = capture_source_binding(repo.path()).unwrap();
        assert_ne!(product_changed.tree_digest, baseline.tree_digest);
        assert!(product_changed.dirty);

        fs::write(repo.path().join("product.txt"), "product-v1\n").unwrap();
        fs::write(
            repo.path().join(".planr/evidence/adapters/adapter.mjs"),
            "process.stdout.write('{\"changed\":true}')\n",
        )
        .unwrap();
        let adapter_changed = capture_source_binding(repo.path()).unwrap();
        assert_ne!(adapter_changed.tree_digest, baseline.tree_digest);
        assert!(adapter_changed.dirty);
    }
}
