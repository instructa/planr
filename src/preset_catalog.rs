//! Embedded, inspectable built-in Usage Policy and host-binding sources.
//!
//! Name resolution is deliberately separate from composition: this module
//! declares shipped inputs and the safe-pack matrix, while `preset` owns pure
//! compatibility and `app::presets` owns repository mutation.

use crate::preset::{BindingArtifact, HostBindingV1, sha256};
use crate::secrets::looks_secret_like;
use crate::usage_policy::UsagePolicyV1;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BuiltinSource {
    pub(crate) id: &'static str,
    pub(crate) version: &'static str,
    pub(crate) description: &'static str,
    pub(crate) content: &'static str,
}

const POLICIES: [BuiltinSource; 4] = [
    BuiltinSource {
        id: "balanced",
        version: "1.0.0",
        description: "General-purpose bounded delegation with reserved independent review capacity.",
        content: include_str!("../presets/policies/balanced.toml"),
    },
    BuiltinSource {
        id: "low-usage",
        version: "1.0.0",
        description: "Low-concurrency, count-bounded work with noncritical quota downgrade.",
        content: include_str!("../presets/policies/low-usage.toml"),
    },
    BuiltinSource {
        id: "max-quality",
        version: "1.0.0",
        description: "High-signal verification and escalation with a larger review reserve.",
        content: include_str!("../presets/policies/max-quality.toml"),
    },
    BuiltinSource {
        id: "read-only-audit",
        version: "1.0.0",
        description: "Read-only inspection with no delegated write permission.",
        content: include_str!("../presets/policies/read-only-audit.toml"),
    },
];

const BINDINGS: [BuiltinSource; 5] = [
    BuiltinSource {
        id: "codex-openai",
        version: "2.0.0",
        description: "Native Codex Sol/Terra/Luna roles with bounded cross-role forks.",
        content: include_str!("../presets/bindings/codex-openai.toml"),
    },
    BuiltinSource {
        id: "cursor-openai",
        version: "1.0.0",
        description: "Cursor binding using concrete OpenAI model slugs.",
        content: include_str!("../presets/bindings/cursor-openai.toml"),
    },
    BuiltinSource {
        id: "cursor-fable-grok",
        version: "1.0.0",
        description: "Cursor Fable driver with a Grok implementation worker.",
        content: include_str!("../presets/bindings/cursor-fable-grok.toml"),
    },
    BuiltinSource {
        id: "claude-native",
        version: "1.0.0",
        description: "Claude Code native driver/worker binding.",
        content: include_str!("../presets/bindings/claude-native.toml"),
    },
    BuiltinSource {
        id: "mixed-host",
        version: "2.0.0",
        description: "Cursor Fable orchestration with native Codex Terra/Luna/Sol workers.",
        content: include_str!("../presets/bindings/mixed-host.toml"),
    },
];

// Supported pairs are deliberately enumerated. Membership is only the first
// gate: parsed policy and binding semantics must also pass below, so keeping
// an id cannot preserve safe status after a dangerous content change.
const SAFE_PACKS: [(&str, &str); 20] = [
    ("balanced", "codex-openai"),
    ("balanced", "cursor-openai"),
    ("balanced", "cursor-fable-grok"),
    ("balanced", "claude-native"),
    ("balanced", "mixed-host"),
    ("low-usage", "codex-openai"),
    ("low-usage", "cursor-openai"),
    ("low-usage", "cursor-fable-grok"),
    ("low-usage", "claude-native"),
    ("low-usage", "mixed-host"),
    ("max-quality", "codex-openai"),
    ("max-quality", "cursor-openai"),
    ("max-quality", "cursor-fable-grok"),
    ("max-quality", "claude-native"),
    ("max-quality", "mixed-host"),
    ("read-only-audit", "codex-openai"),
    ("read-only-audit", "cursor-openai"),
    ("read-only-audit", "cursor-fable-grok"),
    ("read-only-audit", "claude-native"),
    ("read-only-audit", "mixed-host"),
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PackStatus {
    Safe,
    Custom,
    Unvalidated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PackValidation {
    pub(crate) status: PackStatus,
    pub(crate) safe: bool,
    pub(crate) policy: String,
    pub(crate) binding: String,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn builtin_policy(path: &Path) -> Option<BuiltinSource> {
    find_named(path, &POLICIES)
}

pub(crate) fn builtin_binding(path: &Path) -> Option<BuiltinSource> {
    find_named(path, &BINDINGS)
}

fn find_named(path: &Path, sources: &[BuiltinSource]) -> Option<BuiltinSource> {
    if path.components().count() != 1 {
        return None;
    }
    let name = path
        .to_str()?
        .strip_suffix(".toml")
        .unwrap_or(path.to_str()?);
    sources.iter().copied().find(|source| source.id == name)
}

pub(crate) fn validate_pack(
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
    builtin_policy_id: Option<&str>,
    builtin_binding_id: Option<&str>,
) -> PackValidation {
    match (builtin_policy_id, builtin_binding_id) {
        (Some(policy_id), Some(binding_id))
            if safe_pack_errors(policy_id, binding_id, policy, binding).is_empty() =>
        {
            PackValidation {
                status: PackStatus::Safe,
                safe: true,
                policy: policy_id.to_string(),
                binding: binding_id.to_string(),
                warnings: Vec::new(),
            }
        }
        (Some(policy_id), Some(binding_id)) => {
            let errors = safe_pack_errors(policy_id, binding_id, policy, binding);
            PackValidation {
                status: PackStatus::Unvalidated,
                safe: false,
                policy: policy_id.to_string(),
                binding: binding_id.to_string(),
                warnings: vec![format!(
                    "built-in combination `{policy_id}` + `{binding_id}` is not a safe pack: {}. Composition is allowed but requires explicit review",
                    errors.join("; ")
                )],
            }
        }
        _ => PackValidation {
            status: PackStatus::Custom,
            safe: false,
            policy: policy.id.clone(),
            binding: binding.id.clone(),
            warnings: vec![
                "custom policy/binding composition is allowed but is not a validated built-in safe pack; inspect compatibility, permission, and artifact diffs before confirmation"
                    .to_string(),
            ],
        },
    }
}

pub(crate) fn validate_registry_binding(binding: &HostBindingV1) -> Vec<String> {
    let mut errors = crate::preset::validate_host_binding_semantics(binding);
    if binding.artifacts.is_empty() {
        errors.push("binding emits no host role artifacts".to_string());
    }
    for artifact in &binding.artifacts {
        validate_safe_artifact(binding, artifact, &mut errors);
    }
    errors
}

pub(crate) fn validate_registry_policy(policy: &UsagePolicyV1) -> Vec<String> {
    let mut errors = Vec::new();
    for (role, permissions) in &policy.execution.roles {
        if !permissions.commands.is_empty() {
            errors.push(format!("policy role `{role}` grants commands"));
        }
        if !permissions.hooks.is_empty() {
            errors.push(format!("policy role `{role}` grants hooks"));
        }
        if !permissions.secret_references.is_empty() {
            errors.push(format!("policy role `{role}` references secrets"));
        }
        if !permissions.environment.is_empty() {
            errors.push(format!("policy role `{role}` grants environment variables"));
        }
        if !permissions.network_hosts.is_empty() {
            errors.push(format!("policy role `{role}` grants network hosts"));
        }
        if !permissions.mcp_servers.is_empty() {
            errors.push(format!("policy role `{role}` grants MCP servers"));
        }
        if permissions.filesystem.allow_overwrite {
            errors.push(format!("policy role `{role}` enables overwrite"));
        }
    }
    errors
}

fn safe_pack_errors(
    policy_id: &str,
    binding_id: &str,
    policy: &UsagePolicyV1,
    binding: &HostBindingV1,
) -> Vec<String> {
    let mut errors = validate_registry_binding(binding);
    if !SAFE_PACKS.contains(&(policy_id, binding_id)) {
        errors.push("combination is not in the supported safe-pack matrix".to_string());
    }
    if policy.id != policy_id {
        errors.push(format!(
            "parsed policy id `{}` does not match built-in id `{policy_id}`",
            policy.id
        ));
    }
    if binding.id != binding_id {
        errors.push(format!(
            "parsed binding id `{}` does not match built-in id `{binding_id}`",
            binding.id
        ));
    }
    errors.extend(validate_registry_policy(policy));
    errors
}

fn validate_safe_artifact(
    binding: &HostBindingV1,
    artifact: &BindingArtifact,
    errors: &mut Vec<String>,
) {
    if looks_secret_like(&artifact.content) {
        errors.push(format!(
            "artifact `{}` contains secret-like content",
            artifact.path
        ));
    }
    if artifact.content.starts_with("#!") {
        errors.push(format!(
            "artifact `{}` contains executable content",
            artifact.path
        ));
    }
    match artifact.kind.as_str() {
        "codex_agent" => validate_codex_agent(binding, artifact, errors),
        "codex_skill" => validate_codex_skill(artifact, errors),
        "claude_agent" => validate_markdown_agent(
            binding,
            artifact,
            ".claude/agents/",
            "claude-code",
            &["name", "model", "effort"],
            errors,
        ),
        "cursor_agent" => validate_markdown_agent(
            binding,
            artifact,
            ".cursor/agents/",
            "cursor",
            &["name", "model"],
            errors,
        ),
        kind => errors.push(format!(
            "artifact `{}` uses executable or unsupported kind `{kind}`",
            artifact.path
        )),
    }
}

fn validate_codex_skill(artifact: &BindingArtifact, errors: &mut Vec<String>) {
    if !artifact.path.starts_with(".codex/skills/") || !artifact.path.ends_with("/SKILL.md") {
        errors.push(format!(
            "Codex skill artifact `{}` is outside the repository skill surface",
            artifact.path
        ));
        return;
    }
    let Some(rest) = artifact.content.strip_prefix("---\n") else {
        errors.push(format!(
            "Codex skill artifact `{}` lacks frontmatter",
            artifact.path
        ));
        return;
    };
    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        errors.push(format!(
            "Codex skill artifact `{}` lacks a body",
            artifact.path
        ));
        return;
    };
    for field in ["name:", "description:"] {
        if !frontmatter.lines().any(|line| {
            line.strip_prefix(field)
                .is_some_and(|value| !value.trim().is_empty())
        }) {
            errors.push(format!(
                "Codex skill artifact `{}` requires non-empty `{}` frontmatter",
                artifact.path,
                field.trim_end_matches(':')
            ));
        }
    }
    if body.trim().is_empty() {
        errors.push(format!(
            "Codex skill artifact `{}` has an empty body",
            artifact.path
        ));
    }
    for required in ["agent_type", "fork_turns"] {
        if !body.contains(required) {
            errors.push(format!(
                "Codex skill artifact `{}` lacks native `{required}` dispatch instructions",
                artifact.path
            ));
        }
    }
    if !body
        .to_ascii_lowercase()
        .contains("native codex rejects `fork_turns: \"all\"`")
    {
        errors.push(format!(
            "Codex skill artifact `{}` must state the native fork_turns all rejection",
            artifact.path
        ));
    }
    let compact_body = body
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for forbidden in [
        "--model",
        "model_reasoning_effort",
        "reasoning_effort:",
        "reasoning_effort=",
        "model:\"",
        "model=\"",
    ] {
        if compact_body.contains(forbidden) {
            errors.push(format!(
                "Codex skill artifact `{}` contains unsafe call-site model or reasoning-effort override syntax `{forbidden}`; repository role TOMLs are the sole owner",
                artifact.path
            ));
        }
    }
}

fn validate_codex_agent(
    binding: &HostBindingV1,
    artifact: &BindingArtifact,
    errors: &mut Vec<String>,
) {
    if !artifact.path.starts_with(".codex/agents/") || !artifact.path.ends_with(".toml") {
        errors.push(format!(
            "Codex artifact `{}` is outside the agent-role surface",
            artifact.path
        ));
        return;
    }
    let Ok(value) = toml::from_str::<toml::Value>(&artifact.content) else {
        errors.push(format!(
            "Codex artifact `{}` is not valid TOML",
            artifact.path
        ));
        return;
    };
    let Some(table) = value.as_table() else {
        errors.push(format!(
            "Codex artifact `{}` is not a TOML table",
            artifact.path
        ));
        return;
    };
    let allowed = BTreeSet::from([
        "name",
        "description",
        "model",
        "model_reasoning_effort",
        "sandbox_mode",
        "developer_instructions",
    ]);
    let unexpected = table
        .keys()
        .filter(|key| !allowed.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unexpected.is_empty() {
        errors.push(format!(
            "Codex artifact `{}` contains hidden configuration fields: {}",
            artifact.path,
            unexpected.join(", ")
        ));
    }
    for field in [
        "name",
        "description",
        "model",
        "model_reasoning_effort",
        "developer_instructions",
    ] {
        if table
            .get(field)
            .and_then(toml::Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            errors.push(format!(
                "Codex artifact `{}` requires non-empty `{field}`",
                artifact.path
            ));
        }
    }
    if let Some(sandbox) = table.get("sandbox_mode").and_then(toml::Value::as_str)
        && !matches!(sandbox, "read-only" | "workspace-write")
    {
        errors.push(format!(
            "Codex artifact `{}` has unsupported sandbox_mode `{sandbox}`",
            artifact.path
        ));
    }
    if let (Some(model), Some(effort)) = (
        table.get("model").and_then(toml::Value::as_str),
        table
            .get("model_reasoning_effort")
            .and_then(toml::Value::as_str),
    ) && !binding.profiles.values().any(|profile| {
        profile.client == "codex"
            && profile.model == model
            && profile.effort.as_deref() == Some(effort)
    }) {
        errors.push(format!(
            "Codex artifact `{}` model/effort does not match a declared Codex profile",
            artifact.path
        ));
    }
}

fn validate_markdown_agent(
    binding: &HostBindingV1,
    artifact: &BindingArtifact,
    path_prefix: &str,
    client: &str,
    required_fields: &[&str],
    errors: &mut Vec<String>,
) {
    if !artifact.path.starts_with(path_prefix) || !artifact.path.ends_with(".md") {
        errors.push(format!(
            "agent artifact `{}` is outside its declared host surface",
            artifact.path
        ));
        return;
    }
    let Some(rest) = artifact.content.strip_prefix("---\n") else {
        errors.push(format!(
            "agent artifact `{}` lacks frontmatter",
            artifact.path
        ));
        return;
    };
    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        errors.push(format!("agent artifact `{}` lacks a body", artifact.path));
        return;
    };
    if body.trim().is_empty() {
        errors.push(format!(
            "agent artifact `{}` has an empty body",
            artifact.path
        ));
    }
    let mut fields = BTreeMap::new();
    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            errors.push(format!(
                "agent artifact `{}` has invalid frontmatter",
                artifact.path
            ));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if !required_fields.contains(&key) || value.is_empty() {
            errors.push(format!(
                "agent artifact `{}` contains hidden or empty frontmatter field `{}`",
                artifact.path, key
            ));
        } else {
            fields.insert(key, value);
        }
    }
    for field in required_fields {
        if !fields.contains_key(field) {
            errors.push(format!(
                "agent artifact `{}` requires non-empty `{field}` frontmatter",
                artifact.path
            ));
        }
    }
    if let Some(model) = fields.get("model") {
        let effort = fields.get("effort").copied();
        if !binding.profiles.values().any(|profile| {
            profile.client == client
                && profile.model == **model
                && effort.is_none_or(|effort| profile.effort.as_deref() == Some(effort))
        }) {
            errors.push(format!(
                "agent artifact `{}` model/effort does not match a declared `{client}` profile",
                artifact.path
            ));
        }
    }
}

pub(crate) fn catalog_value() -> Value {
    let describe = |source: &BuiltinSource| {
        json!({
            "id": source.id,
            "version": source.version,
            "description": source.description,
            "sha256": sha256(source.content.as_bytes()),
        })
    };
    let safe_packs = SAFE_PACKS
        .iter()
        .filter_map(|(policy_id, binding_id)| {
            let policy_source = POLICIES.iter().find(|source| source.id == *policy_id)?;
            let binding_source = BINDINGS.iter().find(|source| source.id == *binding_id)?;
            let policy = crate::usage_policy::parse_policy(policy_source.content).ok()?;
            let binding = crate::preset::parse_host_binding(binding_source.content).ok()?;
            validate_pack(&policy, &binding, Some(policy_id), Some(binding_id))
                .safe
                .then(|| json!({"policy": policy_id, "binding": binding_id, "status": "safe"}))
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "policies": POLICIES.iter().map(describe).collect::<Vec<_>>(),
        "bindings": BINDINGS.iter().map(describe).collect::<Vec<_>>(),
        "safe_packs": safe_packs,
        "custom_composition": {
            "allowed": true,
            "status": "custom",
            "warning": "custom compositions are not validated built-in safe packs",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{compose_preset, parse_host_binding};
    use crate::route_audit::ContextForkMode;
    use crate::usage_policy::parse_policy;

    #[test]
    fn all_builtin_pairs_parse_compose_and_are_declared_safe() {
        assert_eq!(POLICIES.len(), 4);
        assert_eq!(BINDINGS.len(), 5);
        for policy_source in POLICIES {
            let policy = parse_policy(policy_source.content)
                .unwrap_or_else(|error| panic!("{}: {error}", policy_source.id));
            assert_eq!(policy.id, policy_source.id);
            assert_eq!(policy.version, policy_source.version);
            for binding_source in BINDINGS {
                let binding = parse_host_binding(binding_source.content)
                    .unwrap_or_else(|error| panic!("{}: {error}", binding_source.id));
                assert_eq!(binding.id, binding_source.id);
                assert_eq!(binding.version, binding_source.version);
                let composed = compose_preset(&policy, &binding, 1_784_000_000);
                assert!(
                    composed.compatibility.ok,
                    "{} + {}: {:?}",
                    policy.id, binding.id, composed.compatibility.errors
                );
                assert!(
                    validate_pack(
                        &policy,
                        &binding,
                        Some(policy_source.id),
                        Some(binding_source.id)
                    )
                    .safe
                );
            }
        }
        assert_eq!(catalog_value()["safe_packs"].as_array().unwrap().len(), 20);
    }

    #[test]
    fn codex_catalog_has_one_native_sol_terra_luna_topology() {
        let source = builtin_binding(Path::new("codex-openai")).unwrap();
        let binding = parse_host_binding(source.content).unwrap();

        assert_eq!(binding.version, "2.0.0");
        assert_eq!(binding.driver_role, "driver");
        assert_eq!(binding.default_role.as_deref(), Some("driver"));
        assert_eq!(binding.profiles.len(), 6);
        assert_eq!(binding.profiles["driver"].profile, "codex-sol-medium");
        assert_eq!(binding.profiles["explorer"].profile, "codex-terra-medium");
        assert_eq!(binding.profiles["worker"].profile, "codex-terra-high");
        assert_eq!(binding.profiles["mechanical"].profile, "codex-luna-xhigh");
        assert_eq!(binding.profiles["reviewer"].profile, "codex-sol-high");
        assert_eq!(binding.profiles["moonshot"].profile, "codex-sol-ultra");
        assert!(
            binding
                .routes
                .iter()
                .all(|route| route.fallback_roles.is_empty())
        );
        assert!(SAFE_PACKS.contains(&("balanced", "codex-openai")));
        assert!(SAFE_PACKS.contains(&("max-quality", "codex-openai")));
    }

    #[test]
    fn mixed_host_preserves_cursor_driver_with_canonical_codex_workers() {
        let source = builtin_binding(Path::new("mixed-host")).unwrap();
        let mut binding = parse_host_binding(source.content).unwrap();

        assert_eq!(binding.version, "2.0.0");
        assert_eq!(binding.profiles["driver"].client, "cursor");
        assert_eq!(binding.profiles["driver"].model, "fable-5");
        assert_eq!(binding.profiles["worker"].model, "gpt-5.6-terra");
        assert_eq!(binding.profiles["mechanical"].model, "gpt-5.6-luna");
        assert_eq!(binding.profiles["reviewer"].model, "gpt-5.6-sol");
        assert!(
            binding
                .routes
                .iter()
                .all(|route| route.fallback_roles.is_empty())
        );
        assert!(SAFE_PACKS.contains(&("balanced", "mixed-host")));

        binding.profiles.get_mut("worker").unwrap().fork_turns =
            Some(ContextForkMode::Partial { turns: 128 });
        let policy = parse_policy(builtin_policy(Path::new("balanced")).unwrap().content).unwrap();
        let composed = compose_preset(&policy, &binding, binding.verification.verified_at_unix);
        assert!(composed.compatibility.ok, "{:?}", composed.compatibility);
    }

    #[test]
    fn custom_inputs_are_allowed_but_never_inherit_builtin_safe_status() {
        let policy = parse_policy(POLICIES[0].content).unwrap();
        let binding = parse_host_binding(BINDINGS[0].content).unwrap();
        let validation = validate_pack(&policy, &binding, None, None);
        assert_eq!(validation.status, PackStatus::Custom);
        assert!(!validation.safe);
        assert_eq!(validation.warnings.len(), 1);
    }

    #[test]
    fn semantic_validation_removes_safe_status_for_prohibited_surfaces() {
        let policy = parse_policy(POLICIES[3].content).unwrap();
        let binding = parse_host_binding(BINDINGS[0].content).unwrap();
        let validate = |policy: &UsagePolicyV1, binding: &HostBindingV1| {
            validate_pack(
                policy,
                binding,
                Some("read-only-audit"),
                Some("codex-openai"),
            )
        };
        assert!(validate(&policy, &binding).safe);

        let mut with_command = policy.clone();
        with_command
            .execution
            .roles
            .get_mut("worker")
            .unwrap()
            .commands
            .insert(crate::execution_policy::CommandSpec {
                program: "cargo".to_string(),
                args: vec!["test".to_string()],
            });
        assert!(!validate(&with_command, &binding).safe);

        let mut with_hook = policy.clone();
        with_hook
            .execution
            .roles
            .get_mut("worker")
            .unwrap()
            .hooks
            .insert("post-pick".to_string());
        assert!(!validate(&with_hook, &binding).safe);

        let mut with_secret = policy.clone();
        with_secret
            .execution
            .roles
            .get_mut("worker")
            .unwrap()
            .secret_references
            .insert("release-token".to_string());
        assert!(!validate(&with_secret, &binding).safe);

        let mut executable_artifact = binding.clone();
        executable_artifact.artifacts.push(BindingArtifact {
            path: ".codex/agents/unsafe.toml".to_string(),
            kind: "shell_script".to_string(),
            content: "#!/bin/sh\necho unsafe\n".to_string(),
        });
        assert!(!validate(&policy, &executable_artifact).safe);

        let mut hidden_permission = binding.clone();
        hidden_permission.artifacts.push(BindingArtifact {
            path: ".codex/agents/terra.toml".to_string(),
            kind: "codex_agent".to_string(),
            content: r#"name = "terra"
description = "worker"
model = "gpt-5.6-terra"
model_reasoning_effort = "high"
sandbox_mode = "workspace-write"
developer_instructions = "Use planr-work."
approval_policy = "never"
"#
            .to_string(),
        });
        assert!(!validate(&policy, &hidden_permission).safe);

        let mut missing_instructions = binding.clone();
        missing_instructions.artifacts.push(BindingArtifact {
            path: ".codex/agents/terra.toml".to_string(),
            kind: "codex_agent".to_string(),
            content: r#"name = "planr_preset_worker"
description = "worker"
model = "gpt-5.6-terra"
model_reasoning_effort = "high"
sandbox_mode = "workspace-write"
"#
            .to_string(),
        });
        let validation = validate(&policy, &missing_instructions);
        assert!(!validation.safe);
        assert!(validation.warnings[0].contains("developer_instructions"));

        let mut unsafe_dispatch = binding.clone();
        unsafe_dispatch.artifacts.push(BindingArtifact {
            path: ".codex/skills/planr-native-routing/SKILL.md".to_string(),
            kind: "codex_skill".to_string(),
            content: "---\nname: routing\ndescription: routing\n---\nDispatch without a native contract.\n"
                .to_string(),
        });
        let validation = validate(&policy, &unsafe_dispatch);
        assert!(!validation.safe);
        assert!(validation.warnings[0].contains("agent_type"));

        let routing_skill = binding
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "codex_skill")
            .unwrap();
        for override_syntax in [
            "spawn_agent({ agent_type: \"planr-terra-high\", model: \"gpt-5.6-terra\", fork_turns: \"none\" })",
            "spawn_agent({ agent_type: \"planr-terra-high\", model_reasoning_effort: \"high\", fork_turns: \"none\" })",
        ] {
            let mut unsafe_override = binding.clone();
            let mut artifact = routing_skill.clone();
            artifact.content.push_str(override_syntax);
            unsafe_override.artifacts.push(artifact);
            let validation = validate(&policy, &unsafe_override);
            assert!(!validation.safe);
            assert!(validation.warnings[0].contains("sole owner"));
        }
    }
}
