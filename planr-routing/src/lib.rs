//! Official, optional routing-policy compiler for Planr.
//!
//! This package is the sole owner of named usage policies, model names, host
//! bindings, routing topologies, and generated host artifacts. It emits the
//! provider-neutral `RoutingBundle v1` contract consumed by Planr core.

use anyhow::{Result, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::process::Command;

const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
const GENERATED_AT: &str = "2026-07-16T00:00:00Z";
const GENERATED_AT_UNIX: i64 = 1_784_160_000;
const EVALUATION_SUITE: &str = include_str!("../evaluations/preset-suite-v1.toml");

const POLICIES: [(&str, &str); 4] = [
    ("balanced", include_str!("../usage-policies/balanced.toml")),
    (
        "low-usage",
        include_str!("../usage-policies/low-usage.toml"),
    ),
    (
        "max-quality",
        include_str!("../usage-policies/max-quality.toml"),
    ),
    (
        "read-only-audit",
        include_str!("../usage-policies/read-only-audit.toml"),
    ),
];

const BINDINGS: [(&str, &str); 5] = [
    (
        "codex-openai",
        include_str!("../host-bindings/codex-openai.toml"),
    ),
    (
        "cursor-openai",
        include_str!("../host-bindings/cursor-openai.toml"),
    ),
    (
        "cursor-fable-grok",
        include_str!("../host-bindings/cursor-fable-grok.toml"),
    ),
    (
        "claude-native",
        include_str!("../host-bindings/claude-native.toml"),
    ),
    (
        "mixed-host",
        include_str!("../host-bindings/mixed-host.toml"),
    ),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PolicySource {
    pub policy_id: String,
    pub host: String,
    pub policy_version: String,
    pub binding_id: String,
    pub binding_version: String,
    pub generated_at: String,
    pub requirements: Vec<HostRequirement>,
    pub profiles: BTreeMap<String, Profile>,
    pub routes: Vec<Route>,
    pub route_default: Option<DefaultRoute>,
    pub artifacts: Vec<SourceArtifact>,
    pub evidence: EvaluationEvidence,
    #[serde(skip)]
    usage_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostRequirement {
    pub host: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub client: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Route {
    #[serde(rename = "match")]
    pub selector: RouteSelector,
    pub profile: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DefaultRoute {
    pub profile: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceArtifact {
    pub path: String,
    pub media_type: String,
    pub mode: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluationEvidence {
    #[serde(default)]
    pub evaluation_ids: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingBundleV1 {
    pub schema_version: u32,
    pub bundle_id: String,
    pub policy_id: String,
    pub policy_version: String,
    pub generated_at: String,
    pub source: BundleSource,
    pub requirements: Vec<HostRequirement>,
    pub profiles: BTreeMap<String, Profile>,
    pub routes: Vec<Route>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_default: Option<DefaultRoute>,
    pub artifacts: Vec<BundleArtifact>,
    pub evidence: EvaluationEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleSource {
    pub package: String,
    pub package_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleArtifact {
    pub path: String,
    pub media_type: String,
    pub mode: String,
    pub content: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicySummary {
    pub policy_id: String,
    pub host: String,
    pub policy_version: String,
    pub binding_id: String,
    pub binding_version: String,
    pub profile_count: usize,
    pub artifact_count: usize,
    pub evidence_status: String,
}

#[derive(Debug, Deserialize)]
struct UsagePolicyHeader {
    id: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct HostBinding {
    id: String,
    version: String,
    host: String,
    default_role: Option<String>,
    capabilities: BindingCapabilities,
    profiles: BTreeMap<String, BindingProfile>,
    #[serde(default)]
    routes: Vec<BindingRoute>,
    verification: BindingVerification,
    #[serde(default)]
    artifacts: Vec<BindingArtifact>,
}

#[derive(Debug, Deserialize)]
struct BindingCapabilities {
    model_override: bool,
    effort_override: bool,
    fork_none: bool,
    fork_all: bool,
}

#[derive(Debug, Deserialize)]
struct BindingProfile {
    profile: String,
    client: String,
    model: String,
    agent_type: Option<String>,
    effort: Option<String>,
    cost_tier: Option<String>,
    skill: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BindingRoute {
    work_type: String,
    role: String,
    #[serde(default)]
    fallback_roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BindingVerification {
    id: String,
}

#[derive(Debug, Deserialize)]
struct BindingArtifact {
    path: String,
    kind: String,
    content: String,
}

pub fn list_policies() -> Result<Vec<PolicySummary>> {
    let mut summaries = Vec::new();
    for (policy, _) in POLICIES {
        for (host, _) in BINDINGS {
            let source = show_policy(policy, host)?;
            summaries.push(PolicySummary {
                policy_id: source.policy_id,
                host: source.host,
                policy_version: source.policy_version,
                binding_id: source.binding_id,
                binding_version: source.binding_version,
                profile_count: source.profiles.len(),
                artifact_count: source.artifacts.len() + 2,
                evidence_status: source.evidence.status,
            });
        }
    }
    Ok(summaries)
}

pub fn show_policy(policy: &str, host: &str) -> Result<PolicySource> {
    let policy_raw = POLICIES
        .iter()
        .find(|(id, _)| *id == policy)
        .map(|(_, raw)| *raw)
        .ok_or_else(|| anyhow::anyhow!("unknown routing policy `{policy}`"))?;
    let binding_raw = BINDINGS
        .iter()
        .find(|(id, _)| *id == host)
        .map(|(_, raw)| *raw)
        .ok_or_else(|| anyhow::anyhow!("unknown routing host `{host}`"))?;
    let policy_header: UsagePolicyHeader = toml::from_str(policy_raw)?;
    let binding: HostBinding = toml::from_str(binding_raw)?;

    let profiles = binding
        .profiles
        .values()
        .map(|profile| {
            (
                profile.profile.clone(),
                Profile {
                    client: profile.client.clone(),
                    model: profile.model.clone(),
                    agent_type: profile.agent_type.clone(),
                    effort: profile.effort.clone(),
                    cost_tier: profile.cost_tier.clone(),
                    capabilities: Vec::new(),
                    skill: profile.skill.clone(),
                    notes: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let routes = binding
        .routes
        .iter()
        .map(|route| {
            Ok(Route {
                selector: RouteSelector {
                    work_type: Some(route.work_type.clone()),
                    plan: None,
                },
                profile: binding_profile_id(&binding, &route.role)?.to_string(),
                fallbacks: route
                    .fallback_roles
                    .iter()
                    .map(|role| binding_profile_id(&binding, role).map(ToOwned::to_owned))
                    .collect::<Result<Vec<_>>>()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let route_default = binding
        .default_role
        .as_deref()
        .map(|role| -> Result<DefaultRoute> {
            Ok(DefaultRoute {
                profile: binding_profile_id(&binding, role)?.to_string(),
                fallbacks: Vec::new(),
            })
        })
        .transpose()?;
    let artifacts = binding
        .artifacts
        .into_iter()
        .map(|artifact| SourceArtifact {
            media_type: media_type_for(&artifact.path, &artifact.kind),
            path: artifact.path,
            mode: "create".to_string(),
            content: artifact.content,
        })
        .collect();
    let mut capabilities = Vec::new();
    if binding.capabilities.model_override {
        capabilities.push("model_override".to_string());
    }
    if binding.capabilities.effort_override {
        capabilities.push("reasoning_effort".to_string());
    }
    if binding.capabilities.fork_none {
        capabilities.push("fork_none".to_string());
    }
    if binding.capabilities.fork_all {
        capabilities.push("bounded_context_fork".to_string());
    }
    Ok(PolicySource {
        policy_id: policy_header.id,
        host: host.to_string(),
        policy_version: policy_header.version,
        binding_id: binding.id,
        binding_version: binding.version,
        generated_at: GENERATED_AT.to_string(),
        requirements: vec![HostRequirement {
            host: binding.host,
            capabilities,
        }],
        profiles,
        routes,
        route_default,
        artifacts,
        evidence: EvaluationEvidence {
            evaluation_ids: vec![binding.verification.id],
            status: "experimental".to_string(),
        },
        usage_policy: policy_raw.to_string(),
    })
}

pub fn compile_policy(policy: &str, host: &str) -> Result<RoutingBundleV1> {
    let source = show_policy(policy, host)?;
    validate_source(&source)?;
    let registry = render_registry(&source)?;
    let mut artifacts = vec![
        bundle_artifact(SourceArtifact {
            path: ".planr/agents.toml".to_string(),
            media_type: "application/toml".to_string(),
            mode: "replace".to_string(),
            content: registry,
        }),
        bundle_artifact(SourceArtifact {
            path: ".planr/policy.toml".to_string(),
            media_type: "application/toml".to_string(),
            mode: "replace".to_string(),
            content: source.usage_policy.clone(),
        }),
    ];
    artifacts.extend(source.artifacts.iter().cloned().map(bundle_artifact));
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(RoutingBundleV1 {
        schema_version: 1,
        bundle_id: format!(
            "{}-{}@{}+{}",
            source.policy_id, source.host, source.policy_version, source.binding_version
        ),
        policy_id: source.policy_id,
        policy_version: source.policy_version,
        generated_at: source.generated_at,
        source: BundleSource {
            package: "planr-routing".to_string(),
            package_version: PACKAGE_VERSION.to_string(),
        },
        requirements: source.requirements,
        profiles: source.profiles,
        routes: source.routes,
        route_default: source.route_default,
        artifacts,
        evidence: source.evidence,
    })
}

pub fn compile_json(policy: &str, host: &str) -> Result<String> {
    let mut json = serde_json::to_string_pretty(&compile_policy(policy, host)?)?;
    json.push('\n');
    Ok(json)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeReport {
    pub host: String,
    pub command: Option<String>,
    pub available: bool,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub authentication: String,
    pub limitation: Option<String>,
}

pub fn probe_host(host: &str, command_override: Option<&str>) -> Result<ProbeReport> {
    let source = show_policy("balanced", host)?;
    let requirement = source
        .requirements
        .first()
        .ok_or_else(|| anyhow::anyhow!("binding has no host requirement"))?;
    let default_command = match requirement.host.as_str() {
        "codex" => Some("codex"),
        "cursor" => Some("cursor-agent"),
        "claude-code" => Some("claude"),
        "mixed-host" => None,
        _ => None,
    };
    let command = command_override.or(default_command);
    let (available, version, limitation) = if let Some(command) = command {
        match Command::new(command).arg("--version").output() {
            Ok(output) if output.status.success() => (
                true,
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string()),
                None,
            ),
            Ok(output) => (
                false,
                None,
                Some(format!("version probe exited with {}", output.status)),
            ),
            Err(error) => (false, None, Some(error.to_string())),
        }
    } else {
        (
            false,
            None,
            Some("mixed-host bindings require separate probes for each declared host".to_string()),
        )
    };
    Ok(ProbeReport {
        host: host.to_string(),
        command: command.map(ToOwned::to_owned),
        available,
        version,
        capabilities: requirement.capabilities.clone(),
        authentication: "not_tested".to_string(),
        limitation,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluationReport {
    pub schema_version: u32,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_sha256: String,
    pub policy_id: String,
    pub host: String,
    pub bundle_sha256: String,
    pub scenario_count: usize,
    pub offline_reproducible: bool,
    pub live_evidence: Option<serde_json::Value>,
    pub status: String,
    pub recommended: bool,
}

pub fn evaluate_policy(policy: &str, host: &str) -> Result<EvaluationReport> {
    let suite: toml::Value = toml::from_str(EVALUATION_SUITE)?;
    let suite_id = suite
        .get("id")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("evaluation suite is missing id"))?;
    let suite_version = suite
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("evaluation suite is missing version"))?;
    let scenario_count = suite
        .get("tasks")
        .and_then(toml::Value::as_array)
        .map_or(0, Vec::len);
    let bundle = compile_json(policy, host)?;
    Ok(EvaluationReport {
        schema_version: 1,
        suite_id: suite_id.to_string(),
        suite_version: suite_version.to_string(),
        suite_sha256: sha256(EVALUATION_SUITE.as_bytes()),
        policy_id: policy.to_string(),
        host: show_policy(policy, host)?.host,
        bundle_sha256: sha256(bundle.as_bytes()),
        scenario_count,
        offline_reproducible: scenario_count > 0,
        live_evidence: None,
        status: "experimental".to_string(),
        recommended: false,
    })
}

pub fn catalog_value() -> Result<Value> {
    let mut compositions = Vec::new();
    for summary in list_policies()? {
        let source = show_policy(&summary.policy_id, &summary.host)?;
        let report = evaluate_policy(&summary.policy_id, &summary.host)?;
        let policy: toml::Value = toml::from_str(&source.usage_policy)?;
        let bundle = compile_policy(&summary.policy_id, &summary.host)?;
        let usage = policy
            .get("usage")
            .cloned()
            .unwrap_or_else(|| toml::Value::Table(Default::default()));
        let transitions = policy
            .get("transitions")
            .cloned()
            .unwrap_or_else(|| toml::Value::Table(Default::default()));
        let materiality = policy
            .get("materiality")
            .cloned()
            .unwrap_or_else(|| toml::Value::Table(Default::default()));
        let execution = policy
            .get("execution")
            .cloned()
            .unwrap_or_else(|| toml::Value::Table(Default::default()));
        compositions.push(json!({
            "id": format!("{}-{}@{}+{}", summary.policy_id, summary.binding_id, summary.policy_version, summary.binding_version),
            "entryId": format!("{}-{}", summary.policy_id, summary.binding_id),
            "entryVersion": format!("{}+{}", summary.policy_version, summary.binding_version),
            "status": report.status,
            "statusLabel": "Experimental",
            "recommended": false,
            "freshness": "current",
            "lifecycle": "published",
            "replacement": Value::Null,
            "policy": {
                "id": summary.policy_id,
                "version": summary.policy_version,
                "usage": usage,
                "transitions": transitions,
                "materiality": materiality,
                "execution": execution,
            },
            "binding": {
                "id": summary.binding_id,
                "selector": summary.host,
                "version": summary.binding_version,
                "host": source.requirements.first().map(|requirement| requirement.host.clone()),
                "profiles": bundle.profiles,
                "dispatch": bundle.routes,
            },
            "compatibility": {
                "hosts": source.requirements.iter().map(|requirement| requirement.host.clone()).collect::<Vec<_>>(),
                "minPlanrVersion": "1.4.0",
                "maxPlanrVersion": Value::Null,
            },
            "enforcement": [
                {"dimension": "Repository writes", "state": "verified", "detail": "Core previews and applies only allowlisted repository-local bundle artifacts."},
                {"dimension": "Model and effort", "state": "host_enforced", "detail": "The package generates exact host roles; the host remains execution authority."},
                {"dimension": "Effective route evidence", "state": "unavailable", "detail": "No authenticated live-host evidence is published for this generated catalog entry."}
            ],
            "evaluation": {
                "suiteId": report.suite_id,
                "suiteVersion": report.suite_version,
                "evaluatedAtUnix": GENERATED_AT_UNIX,
                "reviewAtUnix": Value::Null,
                "status": report.status,
                "metrics": {"runs": 0, "oracle_passes": 0, "average_quality_score_bps": Value::Null},
                "thresholds": {},
                "resultHashes": [],
                "fixtureSha256": report.suite_sha256,
            },
            "registry": {
                "id": "planr-routing-official",
                "version": PACKAGE_VERSION,
                "manifestSha256": report.bundle_sha256,
                "signer": Value::Null,
                "signatureVerified": false,
                "trustedMaintainer": false,
                "artifacts": bundle.artifacts.iter().map(|artifact| json!({"path": artifact.path, "sha256": artifact.sha256})).collect::<Vec<_>>(),
            },
            "command": format!("planr-routing compile {} --host {} --output routing-bundle.json && planr routing bundle preview routing-bundle.json", source.policy_id, source.host),
        }));
    }
    Ok(json!({
        "schemaVersion": 1,
        "generatedAtUnix": GENERATED_AT_UNIX,
        "source": {
            "state": "package_generated",
            "entryCount": compositions.len(),
            "trust": "planr_routing_unsigned_catalog_v1",
            "message": "Entries stay experimental until authenticated live evidence and an offline maintainer signature pass."
        },
        "compositions": compositions,
    }))
}

pub fn catalog_json() -> Result<String> {
    let mut output = serde_json::to_string_pretty(&catalog_value()?)?;
    output.push('\n');
    Ok(output)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistrySignature {
    pub algorithm: String,
    pub signer: String,
    pub content_sha256: String,
    pub value: String,
}

pub fn sign_registry(
    content: &[u8],
    signer: &str,
    private_key_hex: &str,
) -> Result<RegistrySignature> {
    if signer.trim().is_empty() {
        bail!("registry signer must not be blank");
    }
    let seed = decode_hex::<32>(private_key_hex.trim()).ok_or_else(|| {
        anyhow::anyhow!("private key file must contain exactly 64 hexadecimal characters")
    })?;
    let key = SigningKey::from_bytes(&seed);
    let signature = key.sign(content);
    Ok(RegistrySignature {
        algorithm: "ed25519".to_string(),
        signer: signer.to_string(),
        content_sha256: sha256(content),
        value: encode_hex(&signature.to_bytes()),
    })
}

pub fn verify_registry_signature(
    content: &[u8],
    signature: &RegistrySignature,
    trusted_signer: &str,
    trusted_public_key_hex: &str,
) -> Result<()> {
    if signature.algorithm != "ed25519" || signature.content_sha256 != sha256(content) {
        bail!("registry signature metadata does not match content");
    }
    if trusted_signer.trim().is_empty() || signature.signer != trusted_signer {
        bail!("registry signature signer does not match the trusted signer");
    }
    let public_key = decode_hex::<32>(trusted_public_key_hex.trim())
        .ok_or_else(|| anyhow::anyhow!("trusted registry public key is invalid"))?;
    let signature_bytes = decode_hex::<64>(&signature.value)
        .ok_or_else(|| anyhow::anyhow!("registry signature value is invalid"))?;
    let key = VerifyingKey::from_bytes(&public_key)?;
    key.verify(content, &Signature::from_bytes(&signature_bytes))?;
    Ok(())
}

fn validate_source(source: &PolicySource) -> Result<()> {
    if source.policy_id.trim().is_empty() || source.host.trim().is_empty() {
        bail!("routing policy id and host must not be blank");
    }
    for route in &source.routes {
        if !source.profiles.contains_key(&route.profile) {
            bail!("route references unknown profile `{}`", route.profile);
        }
    }
    if let Some(default) = &source.route_default
        && !source.profiles.contains_key(&default.profile)
    {
        bail!(
            "default route references unknown profile `{}`",
            default.profile
        );
    }
    if source.evidence.status == "recommended" {
        bail!("policy sources cannot claim recommended without the evaluation gate");
    }
    Ok(())
}

fn binding_profile_id<'a>(binding: &'a HostBinding, role: &str) -> Result<&'a str> {
    binding
        .profiles
        .get(role)
        .map(|profile| profile.profile.as_str())
        .ok_or_else(|| anyhow::anyhow!("binding route references unknown role `{role}`"))
}

fn media_type_for(path: &str, kind: &str) -> String {
    if path.ends_with(".toml") {
        "application/toml"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".md") || kind.ends_with("_skill") || kind.ends_with("_agent") {
        "text/markdown"
    } else {
        "text/plain"
    }
    .to_string()
}

fn render_registry(source: &PolicySource) -> Result<String> {
    #[derive(Serialize)]
    struct Registry<'a> {
        profiles: &'a BTreeMap<String, Profile>,
        routes: &'a [Route],
        #[serde(skip_serializing_if = "Option::is_none")]
        route_default: &'a Option<DefaultRoute>,
    }
    Ok(toml::to_string_pretty(&Registry {
        profiles: &source.profiles,
        routes: &source.routes,
        route_default: &source.route_default,
    })?)
}

fn bundle_artifact(source: SourceArtifact) -> BundleArtifact {
    BundleArtifact {
        sha256: sha256(source.content.as_bytes()),
        path: source.path,
        media_type: source.media_type,
        mode: source.mode,
        content: source.content,
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_policy_binding_pool_compiles_deterministically() {
        let summaries = list_policies().unwrap();
        assert_eq!(summaries.len(), 20);
        for summary in summaries {
            let first = compile_json(&summary.policy_id, &summary.host).unwrap();
            let second = compile_json(&summary.policy_id, &summary.host).unwrap();
            assert_eq!(first, second);
            assert!(first.contains(".planr/policy.toml"));
        }
    }

    #[test]
    fn codex_and_mixed_bindings_keep_native_bounded_fork_topology() {
        let codex = compile_json("balanced", "codex-openai").unwrap();
        let mixed = compile_json("balanced", "mixed-host").unwrap();
        assert!(codex.contains("gpt-5.6-sol"));
        assert!(codex.contains("fork_turns: \\\"none\\\""));
        assert!(mixed.contains("fable-5"));
        assert!(mixed.contains("gpt-5.6-terra"));
        assert_ne!(codex, mixed);
    }

    #[test]
    fn codex_agent_types_match_registered_toml_names() {
        for host in ["codex-openai", "mixed-host"] {
            let bundle = compile_policy("balanced", host).unwrap();
            let registered_names = bundle
                .artifacts
                .iter()
                .filter(|artifact| artifact.path.starts_with(".codex/agents/"))
                .map(|artifact| {
                    toml::from_str::<toml::Value>(&artifact.content).unwrap()["name"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect::<std::collections::BTreeSet<_>>();
            let skill = bundle
                .artifacts
                .iter()
                .find(|artifact| artifact.path.ends_with("planr-native-routing/SKILL.md"));
            for profile in bundle
                .profiles
                .values()
                .filter(|profile| profile.client == "codex")
            {
                let agent_type = profile.agent_type.as_deref().unwrap();
                assert!(registered_names.contains(agent_type));
                assert!(skill.is_some_and(|artifact| artifact.content.contains(agent_type)));
            }
        }
    }

    #[test]
    fn generated_registry_is_derived_from_binding_profiles_and_routes() {
        for host in BINDINGS.map(|(host, _)| host) {
            let bundle = compile_policy("balanced", host).unwrap();
            let registry = bundle
                .artifacts
                .iter()
                .find(|artifact| artifact.path == ".planr/agents.toml")
                .unwrap();
            let parsed: toml::Value = toml::from_str(&registry.content).unwrap();
            assert_eq!(
                parsed["profiles"].as_table().unwrap().len(),
                bundle.profiles.len()
            );
            assert_eq!(
                parsed["routes"].as_array().unwrap().len(),
                bundle.routes.len()
            );
        }
    }

    #[test]
    fn checked_in_contract_fixtures_are_generated_outputs() {
        for (host, fixture) in [
            (
                "codex-openai",
                include_str!("../fixtures/routing-bundle-v1/valid-balanced-codex.json"),
            ),
            (
                "mixed-host",
                include_str!("../fixtures/routing-bundle-v1/valid-balanced-mixed.json"),
            ),
        ] {
            let generated: serde_json::Value =
                serde_json::from_str(&compile_json("balanced", host).unwrap()).unwrap();
            let checked_in: serde_json::Value = serde_json::from_str(fixture).unwrap();
            assert_eq!(generated, checked_in, "regenerate fixture for {host}");
        }
    }

    #[test]
    fn offline_evaluation_never_claims_live_verification_or_recommendation() {
        let report = evaluate_policy("balanced", "codex-openai").unwrap();
        assert!(report.offline_reproducible);
        assert!(report.scenario_count >= 7);
        assert_eq!(report.status, "experimental");
        assert!(!report.recommended);
    }

    #[test]
    fn no_in_memory_claim_can_promote_offline_evaluation() {
        let report = evaluate_policy("balanced", "codex-openai").unwrap();
        assert!(report.live_evidence.is_none());
        assert_eq!(report.status, "experimental");
        assert!(!report.recommended);
    }

    #[test]
    fn catalog_is_reproducible_and_contains_the_full_pool() {
        let first = catalog_json().unwrap();
        let second = catalog_json().unwrap();
        assert_eq!(first, second);
        let value: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(value["compositions"].as_array().unwrap().len(), 20);
        assert!(
            value["compositions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["recommended"] == false)
        );
    }

    #[test]
    fn registry_signatures_are_content_bound() {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let trusted_public_key = encode_hex(signing_key.verifying_key().as_bytes());
        let signature = sign_registry(b"catalog", "fixture", &"07".repeat(32)).unwrap();
        verify_registry_signature(b"catalog", &signature, "fixture", &trusted_public_key).unwrap();
        assert!(
            verify_registry_signature(b"tampered", &signature, "fixture", &trusted_public_key)
                .is_err()
        );
        let attacker_key = encode_hex(
            SigningKey::from_bytes(&[8_u8; 32])
                .verifying_key()
                .as_bytes(),
        );
        assert!(
            verify_registry_signature(b"catalog", &signature, "fixture", &attacker_key).is_err()
        );
        assert!(
            verify_registry_signature(b"catalog", &signature, "attacker", &trusted_public_key)
                .is_err()
        );
    }

    #[test]
    fn probe_does_not_infer_authentication_from_version_availability() {
        let report =
            probe_host("codex-openai", Some("definitely-not-a-planr-host-command")).unwrap();
        assert!(!report.available);
        assert_eq!(report.authentication, "not_tested");
    }
}
