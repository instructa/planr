//! Pure policy-preset and host-binding composition.
//!
//! Usage policy remains provider-neutral. Exact host, model, effort, and
//! dispatch-context behavior lives in a versioned binding. This module only
//! validates and composes values; repository writes belong to the application
//! preset service.

use crate::agents::{AgentProfile, AgentRegistry, DefaultRoute, Route, RouteSelector};
use crate::execution_policy::{ApprovalKind, CommandSpec, RolePermissions};
use crate::route_audit::ContextForkMode;
use crate::secrets::{looks_secret_like, redact_secrets};
use crate::usage_policy::{UsagePolicyV1, validate_policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const HOST_BINDING_SCHEMA_VERSION: u32 = 1;
pub const PRESET_LOCK_SCHEMA_VERSION: u32 = 1;
pub const ACTIVE_POLICY_PATH: &str = ".planr/policy.toml";
pub const ACTIVE_REGISTRY_PATH: &str = ".planr/agents.toml";
pub const PRESET_LOCK_PATH: &str = ".planr/preset.lock.toml";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBindingV1 {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub host: String,
    pub driver_role: String,
    pub capabilities: HostCapabilities,
    #[serde(default)]
    pub capability_evidence: Vec<String>,
    pub profiles: BTreeMap<String, BoundProfile>,
    #[serde(default)]
    pub routes: Vec<BindingRoute>,
    #[serde(default)]
    pub default_role: Option<String>,
    #[serde(default)]
    pub billing_assumptions: Vec<String>,
    #[serde(default)]
    pub known_limitations: Vec<String>,
    pub verification: BindingVerification,
    #[serde(default)]
    pub artifacts: Vec<BindingArtifact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilities {
    pub model_override: bool,
    pub effort_override: bool,
    pub fork_none: bool,
    pub fork_all: bool,
    #[serde(default)]
    pub max_partial_fork_turns: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundProfile {
    pub profile: String,
    pub client: String,
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub cost_tier: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub fork_turns: Option<ContextForkMode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingRoute {
    pub work_type: String,
    pub role: String,
    #[serde(default)]
    pub fallback_roles: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingVerification {
    pub id: String,
    pub verified_at_unix: u64,
    pub max_age_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingArtifact {
    pub path: String,
    pub kind: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComposedPreset {
    pub policy_id: String,
    pub policy_version: String,
    pub binding_id: String,
    pub binding_version: String,
    pub host: String,
    pub profiles: BTreeMap<String, String>,
    pub dispatch: BTreeMap<String, DispatchContext>,
    pub registry: AgentRegistry,
    pub compatibility: CompatibilityReport,
    pub verification_age: VerificationAge,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DispatchContext {
    pub model_override: bool,
    pub effort_override: bool,
    pub fork_turns: ContextForkMode,
    pub capability_evidence: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompatibilityReport {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerificationAge {
    pub verification_id: String,
    pub verified_at_unix: u64,
    pub age_seconds: u64,
    pub max_age_seconds: u64,
    pub status: VerificationStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Fresh,
    Stale,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RolePermissionAdditions {
    pub read_roots: BTreeSet<String>,
    pub write_roots: BTreeSet<String>,
    pub network_hosts: BTreeSet<String>,
    pub tools: BTreeSet<String>,
    pub mcp_servers: BTreeSet<String>,
    pub commands: BTreeSet<CommandSpec>,
    pub environment: BTreeSet<String>,
    pub hooks: BTreeSet<String>,
    pub secret_references: BTreeSet<String>,
    pub approvals: BTreeSet<ApprovalKind>,
    pub allow_overwrite_enabled: bool,
}

impl RolePermissionAdditions {
    pub fn is_empty(&self) -> bool {
        self.read_roots.is_empty()
            && self.write_roots.is_empty()
            && self.network_hosts.is_empty()
            && self.tools.is_empty()
            && self.mcp_servers.is_empty()
            && self.commands.is_empty()
            && self.environment.is_empty()
            && self.hooks.is_empty()
            && self.secret_references.is_empty()
            && self.approvals.is_empty()
            && !self.allow_overwrite_enabled
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LockedSource {
    pub id: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PresetLock {
    pub schema_version: u32,
    pub policy: LockedSource,
    pub binding: LockedSource,
    pub planr_version: String,
    pub verification_id: String,
    pub verification_status: VerificationStatus,
    pub applied_at: String,
    pub artifact_hashes: BTreeMap<String, String>,
    pub local_modifications: Vec<String>,
}

pub fn parse_host_binding(text: &str) -> Result<HostBindingV1, String> {
    toml::from_str(text).map_err(|error| format!("host binding parse failed: {error}"))
}

pub fn compose_preset(
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    now_unix: u64,
) -> ComposedPreset {
    let secret_metadata_errors = validate_secret_metadata(binding);
    if !secret_metadata_errors.is_empty() {
        return rejected_secret_metadata_composition(
            policy,
            binding,
            now_unix,
            secret_metadata_errors,
        );
    }

    let mut errors = validate_binding(binding);
    errors.extend(validate_policy(policy).into_iter().map(|diagnostic| {
        format!(
            "policy.{}: {}",
            diagnostic.field.as_deref().unwrap_or("root"),
            diagnostic.message
        )
    }));

    let driver = binding.profiles.get(&binding.driver_role);
    let mut profiles = BTreeMap::new();
    let mut dispatch = BTreeMap::new();
    let mut registry = AgentRegistry::default();

    for role in policy.execution.roles.keys() {
        if !binding.profiles.contains_key(role) {
            errors.push(format!(
                "binding.profiles is missing policy execution role `{role}`"
            ));
        }
    }

    for (role, profile) in &binding.profiles {
        profiles.insert(role.clone(), profile.profile.clone());
        if registry.profiles.contains_key(&profile.profile) {
            errors.push(format!(
                "binding profile id `{}` is assigned to more than one role",
                profile.profile
            ));
        }
        registry.profiles.insert(
            profile.profile.clone(),
            AgentProfile {
                client: profile.client.clone(),
                model: profile.model.clone(),
                effort: profile.effort.clone(),
                cost_tier: profile.cost_tier.clone(),
                capabilities: profile.capabilities.clone(),
                skill: profile.skill.clone(),
                notes: profile.notes.clone(),
            },
        );

        let model_override = driver.is_some_and(|driver| driver.model != profile.model);
        let effort_override = driver.is_some_and(|driver| driver.effort != profile.effort);
        if model_override && !binding.capabilities.model_override {
            errors.push(format!(
                "binding role `{role}` changes model but model_override is unsupported"
            ));
        }
        if effort_override && !binding.capabilities.effort_override {
            errors.push(format!(
                "binding role `{role}` changes effort but effort_override is unsupported"
            ));
        }

        let fork_turns = profile.fork_turns.clone().unwrap_or(ContextForkMode::None);
        validate_fork(
            binding,
            role,
            model_override || effort_override,
            &fork_turns,
            &mut errors,
        );
        dispatch.insert(
            role.clone(),
            DispatchContext {
                model_override,
                effort_override,
                fork_turns,
                capability_evidence: binding.capability_evidence.clone(),
            },
        );
    }

    for route in &binding.routes {
        let Some(profile) = binding.profiles.get(&route.role) else {
            continue;
        };
        let fallbacks = route
            .fallback_roles
            .iter()
            .filter_map(|role| binding.profiles.get(role))
            .map(|profile| profile.profile.clone())
            .collect();
        registry.routes.push(Route {
            selector: RouteSelector {
                work_type: Some(route.work_type.clone()),
                plan: None,
            },
            profile: profile.profile.clone(),
            fallbacks,
        });
    }
    registry.route_default = binding.default_role.as_ref().and_then(|role| {
        binding.profiles.get(role).map(|profile| DefaultRoute {
            profile: profile.profile.clone(),
            fallbacks: Vec::new(),
        })
    });

    let age_seconds = now_unix.saturating_sub(binding.verification.verified_at_unix);
    let verification_status = if age_seconds > binding.verification.max_age_seconds {
        VerificationStatus::Stale
    } else {
        VerificationStatus::Fresh
    };
    let mut warnings = binding.known_limitations.clone();
    warnings.extend(
        binding
            .billing_assumptions
            .iter()
            .map(|value| format!("billing assumption: {value}")),
    );
    if verification_status == VerificationStatus::Stale {
        warnings.push(format!(
            "binding verification `{}` is stale",
            binding.verification.id
        ));
    }

    ComposedPreset {
        policy_id: policy.id.clone(),
        policy_version: policy.version.clone(),
        binding_id: binding.id.clone(),
        binding_version: binding.version.clone(),
        host: binding.host.clone(),
        profiles,
        dispatch,
        registry,
        compatibility: CompatibilityReport {
            ok: errors.is_empty(),
            errors,
            warnings,
        },
        verification_age: VerificationAge {
            verification_id: binding.verification.id.clone(),
            verified_at_unix: binding.verification.verified_at_unix,
            age_seconds,
            max_age_seconds: binding.verification.max_age_seconds,
            status: verification_status,
        },
    }
}

pub(crate) fn validate_host_binding_semantics(binding: &HostBindingV1) -> Vec<String> {
    let mut errors = validate_secret_metadata(binding);
    errors.extend(validate_binding(binding));
    errors
}

fn rejected_secret_metadata_composition(
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    now_unix: u64,
    errors: Vec<String>,
) -> ComposedPreset {
    let age_seconds = now_unix.saturating_sub(binding.verification.verified_at_unix);
    let status = if age_seconds > binding.verification.max_age_seconds {
        VerificationStatus::Stale
    } else {
        VerificationStatus::Fresh
    };
    ComposedPreset {
        policy_id: policy.id.clone(),
        policy_version: policy.version.clone(),
        binding_id: redact_metadata(&binding.id),
        binding_version: redact_metadata(&binding.version),
        host: redact_metadata(&binding.host),
        profiles: BTreeMap::new(),
        dispatch: BTreeMap::new(),
        registry: AgentRegistry::default(),
        compatibility: CompatibilityReport {
            ok: false,
            errors,
            warnings: Vec::new(),
        },
        verification_age: VerificationAge {
            verification_id: redact_metadata(&binding.verification.id),
            verified_at_unix: binding.verification.verified_at_unix,
            age_seconds,
            max_age_seconds: binding.verification.max_age_seconds,
            status,
        },
    }
}

fn redact_metadata(value: &str) -> String {
    redact_secrets(value).unwrap_or_else(|| value.to_string())
}

fn validate_secret_metadata(binding: &HostBindingV1) -> Vec<String> {
    let mut errors = Vec::new();
    let mut check = |field: String, value: &str| {
        if looks_secret_like(value) {
            errors.push(format!(
                "{field} contains secret-like metadata and is forbidden"
            ));
        }
    };

    for (field, value) in [
        ("binding.id", binding.id.as_str()),
        ("binding.version", binding.version.as_str()),
        ("binding.host", binding.host.as_str()),
        ("binding.driver_role", binding.driver_role.as_str()),
        ("binding.verification.id", binding.verification.id.as_str()),
    ] {
        check(field.to_string(), value);
    }
    if let Some(default_role) = &binding.default_role {
        check("binding.default_role".to_string(), default_role);
    }
    for (index, value) in binding.capability_evidence.iter().enumerate() {
        check(format!("binding.capability_evidence[{index}]"), value);
    }
    for (index, value) in binding.billing_assumptions.iter().enumerate() {
        check(format!("binding.billing_assumptions[{index}]"), value);
    }
    for (index, value) in binding.known_limitations.iter().enumerate() {
        check(format!("binding.known_limitations[{index}]"), value);
    }
    for (index, (role, profile)) in binding.profiles.iter().enumerate() {
        let base = format!("binding.profiles[{index}]");
        check(format!("{base}.role"), role);
        check(format!("{base}.profile"), &profile.profile);
        check(format!("{base}.client"), &profile.client);
        check(format!("{base}.model"), &profile.model);
        if let Some(effort) = &profile.effort {
            check(format!("{base}.effort"), effort);
        }
        if let Some(cost_tier) = &profile.cost_tier {
            check(format!("{base}.cost_tier"), cost_tier);
        }
        for (capability_index, capability) in profile.capabilities.iter().enumerate() {
            check(
                format!("{base}.capabilities[{capability_index}]"),
                capability,
            );
        }
        if let Some(skill) = &profile.skill {
            check(format!("{base}.skill"), skill);
        }
        if let Some(notes) = &profile.notes {
            check(format!("{base}.notes"), notes);
        }
    }
    for (index, route) in binding.routes.iter().enumerate() {
        let base = format!("binding.routes[{index}]");
        check(format!("{base}.work_type"), &route.work_type);
        check(format!("{base}.role"), &route.role);
        for (fallback_index, role) in route.fallback_roles.iter().enumerate() {
            check(format!("{base}.fallback_roles[{fallback_index}]"), role);
        }
    }
    for (index, artifact) in binding.artifacts.iter().enumerate() {
        check(format!("binding.artifacts[{index}].path"), &artifact.path);
        check(format!("binding.artifacts[{index}].kind"), &artifact.kind);
    }
    errors
}

fn validate_binding(binding: &HostBindingV1) -> Vec<String> {
    let mut errors = Vec::new();
    if binding.schema_version != HOST_BINDING_SCHEMA_VERSION {
        errors.push(format!(
            "binding.schema_version must be {HOST_BINDING_SCHEMA_VERSION}"
        ));
    }
    for (field, value) in [
        ("binding.id", binding.id.as_str()),
        ("binding.version", binding.version.as_str()),
        ("binding.host", binding.host.as_str()),
        ("binding.driver_role", binding.driver_role.as_str()),
        ("binding.verification.id", binding.verification.id.as_str()),
    ] {
        if value.trim().is_empty() {
            errors.push(format!("{field} must not be blank"));
        }
    }
    if binding.verification.max_age_seconds == 0 {
        errors.push("binding.verification.max_age_seconds must be positive".to_string());
    }
    if !binding.profiles.contains_key(&binding.driver_role) {
        errors.push(format!(
            "binding.driver_role `{}` is not declared in profiles",
            binding.driver_role
        ));
    }
    for (role, profile) in &binding.profiles {
        if !valid_id(role) {
            errors.push(format!("binding profile role `{role}` is not a valid id"));
        }
        for (field, value) in [
            ("profile", profile.profile.as_str()),
            ("client", profile.client.as_str()),
            ("model", profile.model.as_str()),
        ] {
            if value.trim().is_empty() {
                errors.push(format!("binding.profiles.{role}.{field} must not be blank"));
            }
        }
        if binding.host != "mixed-host" && profile.client != binding.host {
            errors.push(format!(
                "binding.profiles.{role}.client `{}` must match host `{}`; use host `mixed-host` for explicit cross-client bindings",
                profile.client, binding.host
            ));
        }
    }
    for (index, evidence) in binding.capability_evidence.iter().enumerate() {
        if evidence.trim().is_empty() {
            errors.push(format!(
                "binding.capability_evidence[{index}] must not be blank"
            ));
        }
    }
    for route in &binding.routes {
        if route.work_type.trim().is_empty() {
            errors.push("binding route work_type must not be blank".to_string());
        }
        for role in std::iter::once(&route.role).chain(&route.fallback_roles) {
            if !binding.profiles.contains_key(role) {
                errors.push(format!("binding route references unknown role `{role}`"));
            }
        }
    }
    if let Some(role) = &binding.default_role
        && !binding.profiles.contains_key(role)
    {
        errors.push(format!(
            "binding default_role references unknown role `{role}`"
        ));
    }
    errors
}

fn validate_fork(
    binding: &HostBindingV1,
    role: &str,
    cross_tier: bool,
    fork_turns: &ContextForkMode,
    errors: &mut Vec<String>,
) {
    match fork_turns {
        ContextForkMode::None if !binding.capabilities.fork_none => errors.push(format!(
            "binding role `{role}` requests fork_turns none but the host does not support it"
        )),
        ContextForkMode::All if !binding.capabilities.fork_all => errors.push(format!(
            "binding role `{role}` requests fork_turns all but the host does not support it"
        )),
        ContextForkMode::All if binding.host == "codex" && cross_tier => errors.push(format!(
            "Codex cross-tier role `{role}` cannot use fork_turns all because it defeats model/effort overrides; use none or an evidenced positive partial fork"
        )),
        ContextForkMode::Partial { turns } => {
            if *turns == 0 {
                errors.push(format!(
                    "binding role `{role}` partial fork turns must be positive"
                ));
            }
            if binding
                .capabilities
                .max_partial_fork_turns
                .is_none_or(|maximum| *turns > maximum)
            {
                errors.push(format!(
                    "binding role `{role}` partial fork exceeds the host capability"
                ));
            }
            if !binding
                .capability_evidence
                .iter()
                .any(|evidence| !evidence.trim().is_empty())
            {
                errors.push(format!(
                    "binding role `{role}` partial fork requires explicit capability evidence"
                ));
            }
        }
        _ => {}
    }
}

pub fn permission_additions(
    current: Option<&UsagePolicyV1>,
    proposed: &UsagePolicyV1,
) -> BTreeMap<String, RolePermissionAdditions> {
    let mut additions = BTreeMap::new();
    for (role, next) in &proposed.execution.roles {
        let previous = current.and_then(|policy| policy.execution.roles.get(role));
        let diff = permission_additions_for_role(previous, next);
        if !diff.is_empty() {
            additions.insert(role.clone(), diff);
        }
    }
    additions
}

fn permission_additions_for_role(
    current: Option<&RolePermissions>,
    proposed: &RolePermissions,
) -> RolePermissionAdditions {
    let empty = RolePermissions::default();
    let current = current.unwrap_or(&empty);
    RolePermissionAdditions {
        read_roots: difference(
            &proposed.filesystem.read_roots,
            &current.filesystem.read_roots,
        ),
        write_roots: difference(
            &proposed.filesystem.write_roots,
            &current.filesystem.write_roots,
        ),
        network_hosts: difference(&proposed.network_hosts, &current.network_hosts),
        tools: difference(&proposed.tools, &current.tools),
        mcp_servers: difference(&proposed.mcp_servers, &current.mcp_servers),
        commands: difference(&proposed.commands, &current.commands),
        environment: difference(&proposed.environment, &current.environment),
        hooks: difference(&proposed.hooks, &current.hooks),
        secret_references: difference(&proposed.secret_references, &current.secret_references),
        approvals: difference(&proposed.approvals, &current.approvals),
        allow_overwrite_enabled: proposed.filesystem.allow_overwrite
            && !current.filesystem.allow_overwrite,
    }
}

fn difference<T: Clone + Ord>(next: &BTreeSet<T>, current: &BTreeSet<T>) -> BTreeSet<T> {
    next.difference(current).cloned().collect()
}

pub fn sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_policy::{ExecutionPolicy, FilesystemPermissions};
    use crate::usage_policy::{
        AvailabilityFallbackPolicy, BudgetExhaustionBehavior, MaterialityPolicy, MeteringMode,
        QualityEscalationPolicy, QuotaDowngradePolicy, RetryPolicy, SafetyStopPolicy,
        TransitionPolicy, UsageLimits,
    };

    fn policy() -> UsagePolicyV1 {
        UsagePolicyV1 {
            schema_version: 1,
            id: "portable".to_string(),
            version: "1.0.0".to_string(),
            usage: UsageLimits {
                max_active_agents: 2,
                max_parallel_readers: 2,
                max_parallel_writers: 1,
                max_depth: 1,
                max_attempts: 3,
                max_wall_time_seconds: None,
                max_tool_calls: None,
                max_tokens: None,
                max_credits_micros: None,
                review_reserve_percent: 10,
                budget_exhaustion: BudgetExhaustionBehavior::Stop,
                metering: MeteringMode::Unavailable,
            },
            transitions: TransitionPolicy {
                retry: RetryPolicy {
                    max_same_route_retries: 1,
                },
                availability_fallback: AvailabilityFallbackPolicy {
                    max_fallbacks: 1,
                    require_same_capability_class: true,
                },
                quality_escalation: QualityEscalationPolicy {
                    max_escalations: 1,
                    require_verification_evidence: true,
                },
                quota_downgrade: QuotaDowngradePolicy {
                    enabled: false,
                    max_downgrades: 0,
                    noncritical_only: true,
                },
                safety_stop: SafetyStopPolicy { enabled: true },
            },
            materiality: MaterialityPolicy {
                changed_files_threshold: Some(10),
                changed_lines_threshold: Some(500),
            },
            execution: ExecutionPolicy {
                max_read_scope_entries: 10,
                max_write_scope_entries: 5,
                roles: BTreeMap::from([
                    ("driver".to_string(), RolePermissions::default()),
                    (
                        "worker".to_string(),
                        RolePermissions {
                            filesystem: FilesystemPermissions {
                                read_roots: BTreeSet::from(["src".to_string()]),
                                write_roots: BTreeSet::from(["src".to_string()]),
                                allow_overwrite: false,
                            },
                            ..RolePermissions::default()
                        },
                    ),
                ]),
            },
        }
    }

    fn binding(fork_turns: Option<ContextForkMode>) -> HostBindingV1 {
        HostBindingV1 {
            schema_version: 1,
            id: "codex-local".to_string(),
            version: "1.0.0".to_string(),
            host: "codex".to_string(),
            driver_role: "driver".to_string(),
            capabilities: HostCapabilities {
                model_override: true,
                effort_override: true,
                fork_none: true,
                fork_all: true,
                max_partial_fork_turns: Some(4),
            },
            capability_evidence: vec!["codex-0.138-smoke".to_string()],
            profiles: BTreeMap::from([
                (
                    "driver".to_string(),
                    BoundProfile {
                        profile: "sol".to_string(),
                        client: "codex".to_string(),
                        model: "gpt-5.5".to_string(),
                        effort: Some("xhigh".to_string()),
                        cost_tier: Some("premium".to_string()),
                        capabilities: Vec::new(),
                        skill: None,
                        notes: None,
                        fork_turns: None,
                    },
                ),
                (
                    "worker".to_string(),
                    BoundProfile {
                        profile: "luna".to_string(),
                        client: "codex".to_string(),
                        model: "gpt-5.4-mini".to_string(),
                        effort: Some("high".to_string()),
                        cost_tier: Some("standard".to_string()),
                        capabilities: Vec::new(),
                        skill: Some("planr-work".to_string()),
                        notes: None,
                        fork_turns,
                    },
                ),
            ]),
            routes: vec![BindingRoute {
                work_type: "code".to_string(),
                role: "worker".to_string(),
                fallback_roles: vec!["driver".to_string()],
            }],
            default_role: Some("driver".to_string()),
            billing_assumptions: Vec::new(),
            known_limitations: Vec::new(),
            verification: BindingVerification {
                id: "verify-1".to_string(),
                verified_at_unix: 100,
                max_age_seconds: 100,
            },
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn codex_cross_tier_defaults_to_none_and_rejects_all() {
        let composed = compose_preset(&policy(), &binding(None), 150);
        assert!(composed.compatibility.ok, "{:?}", composed.compatibility);
        assert_eq!(
            composed.dispatch["worker"].fork_turns,
            ContextForkMode::None
        );

        let composed = compose_preset(&policy(), &binding(Some(ContextForkMode::All)), 150);
        assert!(!composed.compatibility.ok);
        assert!(
            composed
                .compatibility
                .errors
                .iter()
                .any(|error| error.contains("cannot use fork_turns all"))
        );
    }

    #[test]
    fn partial_fork_requires_positive_supported_evidenced_turns() {
        let composed = compose_preset(
            &policy(),
            &binding(Some(ContextForkMode::Partial { turns: 2 })),
            150,
        );
        assert!(composed.compatibility.ok);

        let mut unsupported = binding(Some(ContextForkMode::Partial { turns: 5 }));
        unsupported.capability_evidence.clear();
        let composed = compose_preset(&policy(), &unsupported, 150);
        assert!(!composed.compatibility.ok);
        assert_eq!(composed.compatibility.errors.len(), 2);
    }

    #[test]
    fn relabeled_codex_clients_and_blank_partial_evidence_fail_closed() {
        let mut disguised = binding(Some(ContextForkMode::All));
        disguised.profiles.get_mut("worker").unwrap().client = "generic-mcp".to_string();
        let composed = compose_preset(&policy(), &disguised, 150);
        assert!(!composed.compatibility.ok);
        assert!(
            composed
                .compatibility
                .errors
                .iter()
                .any(|error| error.contains("must match host `codex`"))
        );
        assert!(
            composed
                .compatibility
                .errors
                .iter()
                .any(|error| error.contains("cannot use fork_turns all"))
        );

        let mut unproven = binding(Some(ContextForkMode::Partial { turns: 2 }));
        unproven.capability_evidence = vec!["   ".to_string()];
        let composed = compose_preset(&policy(), &unproven, 150);
        assert!(!composed.compatibility.ok);
        assert!(
            composed
                .compatibility
                .errors
                .iter()
                .any(|error| error.contains("must not be blank"))
        );
        assert!(
            composed
                .compatibility
                .errors
                .iter()
                .any(|error| error.contains("requires explicit capability evidence"))
        );
    }

    #[test]
    fn secret_like_binding_metadata_is_rejected_without_serializing_tokens() {
        let aws_secret = "AKIAEXAMPLE123";
        let warning_secret = "xoxb-warning-token";
        let billing_secret = "ghp_billing_token";
        let mut unsafe_binding = binding(None);
        unsafe_binding.capability_evidence = vec![aws_secret.to_string()];
        unsafe_binding.known_limitations = vec![format!("rotate {warning_secret} today")];
        unsafe_binding.billing_assumptions = vec![format!("account {billing_secret}")];

        let composed = compose_preset(&policy(), &unsafe_binding, 150);
        assert!(!composed.compatibility.ok);
        assert!(composed.dispatch.is_empty());
        assert!(composed.registry.profiles.is_empty());
        assert!(composed.compatibility.warnings.is_empty());
        for field in [
            "binding.capability_evidence[0]",
            "binding.known_limitations[0]",
            "binding.billing_assumptions[0]",
        ] {
            assert!(
                composed
                    .compatibility
                    .errors
                    .iter()
                    .any(|error| error.contains(field)),
                "missing safe diagnostic for {field}"
            );
        }
        let serialized = serde_json::to_string(&composed).unwrap();
        for secret in [aws_secret, warning_secret, billing_secret] {
            assert!(
                !serialized.contains(secret),
                "serialized metadata leaked {secret}"
            );
        }
    }

    #[test]
    fn missing_policy_roles_and_permission_additions_are_explicit() {
        let mut binding = binding(None);
        binding.profiles.remove("worker");
        let composed = compose_preset(&policy(), &binding, 250);
        assert!(!composed.compatibility.ok);
        assert_eq!(composed.verification_age.status, VerificationStatus::Stale);

        let additions = permission_additions(None, &policy());
        assert_eq!(
            additions["worker"].write_roots,
            BTreeSet::from(["src".to_string()])
        );
    }
}
