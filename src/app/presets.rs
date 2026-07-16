//! Shared CLI/MCP application service for previewing and applying one policy
//! plus one host binding. All target validation and conflict detection happens
//! before the first write.

use super::App;
use crate::cli::{
    PresetCommand, PresetRegistryCommand, PresetRegistryImportArgs, PresetRegistryVerifyArgs,
};
use crate::preset::{
    ACTIVE_POLICY_PATH, ACTIVE_REGISTRY_PATH, BindingArtifact, ComposedPreset, LockedSource,
    PRESET_LOCK_PATH, PRESET_LOCK_SCHEMA_VERSION, PresetLock, compose_preset, parse_host_binding,
    permission_additions, sha256,
};
use crate::preset_catalog::{
    BuiltinSource, builtin_binding, builtin_policy, catalog_value, validate_pack,
};
use crate::secrets::looks_secret_like;
use crate::usage_policy::{PolicyLoad, load_policy, parse_policy};
use crate::util::{now_string, required_arg};
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MINIMUM_CODEX_VERSION: &str = "0.144.0";

#[derive(Clone, Debug, Default)]
pub(crate) struct CodexCapabilityProbe {
    pub(crate) live_host: Option<crate::preset_eval::LiveHostCommand>,
    pub(crate) trusted_telemetry: Option<crate::preset_eval::TrustedTelemetryCommand>,
}

impl App {
    pub(crate) fn initialize_canonical_codex_agents(&self, force_registry: bool) -> Result<String> {
        let content = canonical_codex_registry_content()?;
        let binding_source =
            builtin_binding(Path::new("codex-openai")).expect("built-in codex-openai binding");
        let binding = parse_host_binding(binding_source.content).map_err(anyhow::Error::msg)?;
        let mut artifacts = binding
            .artifacts
            .iter()
            .map(ProposedArtifact::from_binding)
            .collect::<Vec<_>>();
        artifacts.push(ProposedArtifact::new(
            ACTIVE_REGISTRY_PATH,
            "agent_registry",
            content.clone(),
        ));
        reject_duplicate_targets(&artifacts)?;
        let previews = preview_artifacts(&self.root, &artifacts)?;
        let registry_conflict = previews.iter().any(|preview| {
            preview.path == ACTIVE_REGISTRY_PATH && preview.action == ArtifactAction::Conflict
        });
        let conflicts = previews
            .iter()
            .filter(|preview| {
                preview.action == ArtifactAction::Conflict
                    && (preview.path != ACTIVE_REGISTRY_PATH || !force_registry)
            })
            .map(|preview| preview.path.clone())
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            bail!(
                "agents init refused existing unrelated canonical artifact(s): {}",
                conflicts.join(", ")
            );
        }
        let creates = artifacts
            .iter()
            .zip(&previews)
            .filter(|(_, preview)| preview.action == ArtifactAction::Create)
            .map(|(artifact, _)| ResolvedArtifact {
                target: validated_target(&self.root, &artifact.path)
                    .expect("preview validated repository target"),
                content: artifact.content.clone(),
            })
            .collect::<Vec<_>>();
        apply_artifacts(&creates)?;
        if registry_conflict {
            let registry = validated_target(&self.root, ACTIVE_REGISTRY_PATH)?;
            let temporary = registry.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
            let replace = (|| -> Result<()> {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(content.as_bytes())?;
                file.sync_all()?;
                fs::rename(&temporary, &registry)?;
                Ok(())
            })();
            if let Err(error) = replace {
                let _ = fs::remove_file(&temporary);
                for artifact in &creates {
                    let _ = fs::remove_file(&artifact.target);
                }
                return Err(error);
            }
        }
        Ok(content)
    }

    pub(crate) fn preset(&self, command: PresetCommand) -> Result<()> {
        match command {
            PresetCommand::List(_) => {
                let value = catalog_value();
                self.emit(
                    value,
                    "4 built-in policies, 5 host bindings, 20 declared safe packs".to_string(),
                )
            }
            PresetCommand::Apply(args) => {
                debug_assert!(!(args.preview && args.confirm));
                let probe = codex_capability_probe(
                    &self.root,
                    args.live_host_command,
                    args.live_host_arg,
                    args.trusted_telemetry_signer,
                    args.trusted_telemetry_collector,
                )?;
                let value =
                    self.preset_apply_value(&args.policy, &args.binding, args.confirm, probe)?;
                let action = value["action"].as_str().unwrap_or("previewed").to_string();
                let artifact_count = value["artifacts"].as_array().map_or(0, Vec::len);
                self.emit(
                    value,
                    format!("preset {action}; {artifact_count} repository artifact(s)"),
                )
            }
            PresetCommand::Evaluate(args) => {
                let trusted_telemetry = match (
                    args.trusted_telemetry_signer,
                    args.trusted_telemetry_collector,
                ) {
                    (Some(signer_id), Some(collector)) => {
                        Some(crate::preset_eval::TrustedTelemetryCommand {
                            registry: self.root.join(".planr/trusted-telemetry.toml"),
                            signer_id,
                            collector,
                        })
                    }
                    (None, None) => None,
                    _ => anyhow::bail!(
                        "trusted telemetry signer and collector must be provided together"
                    ),
                };
                let live_host =
                    args.live_host_command
                        .map(|executable| crate::preset_eval::LiveHostCommand {
                            executable,
                            args: args.live_host_arg,
                        });
                let (value, markdown) = self.preset_evaluation_value(
                    args.at_unix,
                    args.host.as_deref(),
                    live_host,
                    trusted_telemetry,
                )?;
                if let Some(report_dir) = args.report_dir.as_deref() {
                    write_evaluation_reports(&self.root, report_dir, &value, &markdown)?;
                }
                self.emit(value, markdown)
            }
            PresetCommand::Registry(args) => self.preset_registry(args.command),
            PresetCommand::TelemetrySign(args) => {
                let mut input = String::new();
                std::io::stdin()
                    .read_to_string(&mut input)
                    .context("failed to read telemetry payload from stdin")?;
                let value =
                    crate::preset_eval::sign_telemetry_payload(&args.private_key_file, &input)
                        .map_err(anyhow::Error::msg)?;
                self.emit(value, "signed telemetry payload".to_string())
            }
        }
    }

    fn preset_registry(&self, command: PresetRegistryCommand) -> Result<()> {
        match command {
            PresetRegistryCommand::Verify(args) => {
                let value = self.preset_registry_verify_value(&args)?;
                let status = value["effective_status"]
                    .as_str()
                    .unwrap_or("verified")
                    .to_string();
                self.emit(value, format!("registry entry verified ({status})"))
            }
            PresetRegistryCommand::Import(args) => {
                debug_assert!(!(args.preview && args.confirm));
                let value = self.preset_registry_import_value(&args)?;
                let action = value["action"].as_str().unwrap_or("preview").to_string();
                let path = value["cache_path"].as_str().unwrap_or_default().to_string();
                self.emit(value, format!("registry entry {action}: {path}"))
            }
            PresetRegistryCommand::List(args) => {
                let value = crate::preset_registry::list_cache(
                    &self.root,
                    args.at_unix
                        .unwrap_or_else(crate::preset_registry::now_unix),
                )
                .map_err(anyhow::Error::msg)?;
                let count = value["entries"].as_array().map_or(0, Vec::len);
                self.emit(value, format!("{count} cached registry entry(s)"))
            }
        }
    }

    pub(crate) fn preset_registry_verify_value(
        &self,
        args: &PresetRegistryVerifyArgs,
    ) -> Result<Value> {
        let manifest_raw = fs::read_to_string(&args.manifest).with_context(|| {
            format!(
                "failed to read registry manifest {}",
                args.manifest.display()
            )
        })?;
        let trust_store = load_registry_trust_store(&self.root, args.trust_store.as_deref())?;
        let verified = crate::preset_registry::verify_entry(
            &manifest_raw,
            &args.entry,
            &args.content_root,
            trust_store.as_ref(),
            args.at_unix
                .unwrap_or_else(crate::preset_registry::now_unix),
            env!("CARGO_PKG_VERSION"),
            args.host.as_deref(),
        )
        .map_err(anyhow::Error::msg)?;
        let catalog_preview = registry_catalog_preview(&verified.entry, &args.content_root)?;
        let mut value = serde_json::to_value(verified)?;
        value["catalog_preview"] = catalog_preview;
        Ok(value)
    }

    pub(crate) fn preset_registry_import_value(
        &self,
        args: &PresetRegistryImportArgs,
    ) -> Result<Value> {
        let manifest_raw = fs::read_to_string(&args.manifest).with_context(|| {
            format!(
                "failed to read registry manifest {}",
                args.manifest.display()
            )
        })?;
        let trust_store = load_registry_trust_store(&self.root, args.trust_store.as_deref())?;
        let imported = crate::preset_registry::import_entry(
            &self.root,
            &manifest_raw,
            &args.entry,
            &args.content_root,
            crate::preset_registry::RegistryVerificationOptions {
                trust_store: trust_store.as_ref(),
                now_unix: args
                    .at_unix
                    .unwrap_or_else(crate::preset_registry::now_unix),
                planr_version: env!("CARGO_PKG_VERSION"),
                host: args.host.as_deref(),
            },
            args.confirm,
        )
        .map_err(anyhow::Error::msg)?;
        let value = serde_json::to_value(imported)?;
        if args.confirm {
            self.record_event(
                "preset_registry_imported",
                None,
                json!({
                    "entry_id": args.entry,
                    "cache_path": value["cache_path"],
                    "manifest_sha256": value["entry"]["manifest_sha256"],
                    "effective_status": value["entry"]["effective_status"],
                }),
            )?;
        }
        Ok(value)
    }

    pub(crate) fn preset_registry_verify_mcp_value(&self, args: &Value) -> Result<Value> {
        self.preset_registry_verify_value(&PresetRegistryVerifyArgs {
            manifest: PathBuf::from(required_arg(args, "manifest")?),
            entry: required_arg(args, "entry")?.to_string(),
            content_root: PathBuf::from(required_arg(args, "content_root")?),
            trust_store: optional_registry_string(args, "trust_store")?.map(PathBuf::from),
            at_unix: optional_registry_u64(args, "at_unix")?,
            host: optional_registry_string(args, "host")?,
        })
    }

    pub(crate) fn preset_registry_import_mcp_value(&self, args: &Value) -> Result<Value> {
        self.preset_registry_import_value(&PresetRegistryImportArgs {
            manifest: PathBuf::from(required_arg(args, "manifest")?),
            entry: required_arg(args, "entry")?.to_string(),
            content_root: PathBuf::from(required_arg(args, "content_root")?),
            trust_store: optional_registry_string(args, "trust_store")?.map(PathBuf::from),
            at_unix: optional_registry_u64(args, "at_unix")?,
            host: optional_registry_string(args, "host")?,
            preview: false,
            confirm: args
                .get("confirm")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub(crate) fn preset_registry_list_mcp_value(&self, args: &Value) -> Result<Value> {
        crate::preset_registry::list_cache(
            &self.root,
            optional_registry_u64(args, "at_unix")?
                .unwrap_or_else(crate::preset_registry::now_unix),
        )
        .map_err(anyhow::Error::msg)
    }

    pub(crate) fn preset_evaluation_value(
        &self,
        at_unix: Option<u64>,
        host: Option<&str>,
        live_host: Option<crate::preset_eval::LiveHostCommand>,
        trusted_telemetry: Option<crate::preset_eval::TrustedTelemetryCommand>,
    ) -> Result<(Value, String)> {
        let options = crate::preset_eval::EvaluationOptions {
            at_unix,
            host: host.map(ToOwned::to_owned),
            live_host,
            trusted_telemetry,
        };
        let report =
            crate::preset_eval::evaluate_embedded_suite(&options).map_err(anyhow::Error::msg)?;
        let markdown = crate::preset_eval::render_markdown(&report);
        Ok((
            json!({
                "report": report,
                "report_files": {
                    "machine": "verification.json",
                    "human": "report.md",
                },
            }),
            markdown,
        ))
    }

    pub(crate) fn preset_apply_value(
        &self,
        policy_path: &Path,
        binding_path: &Path,
        confirm: bool,
        codex_probe: CodexCapabilityProbe,
    ) -> Result<Value> {
        match self.build_preset_application(policy_path, binding_path, confirm, &codex_probe) {
            Ok((mut value, application)) => {
                if confirm {
                    apply_artifacts(&application.artifacts)?;
                    self.record_event(
                        "policy_applied",
                        None,
                        json!({
                            "policy": application.lock.policy,
                            "binding": application.lock.binding,
                            "verification_id": application.lock.verification_id,
                            "permission_diff": application.permission_diff,
                            "artifacts": application.previews,
                        }),
                    )?;
                    value["action"] = json!("applied");
                    value["mutation"] = json!(true);
                }
                Ok(value)
            }
            Err(error) => {
                let _ = self.record_event(
                    "policy_apply_rejected",
                    None,
                    json!({"reason": error.to_string()}),
                );
                Err(error)
            }
        }
    }

    fn build_preset_application(
        &self,
        policy_path: &Path,
        binding_path: &Path,
        confirm: bool,
        codex_probe: &CodexCapabilityProbe,
    ) -> Result<(Value, PresetApplication)> {
        let policy_source = load_policy_source(&self.root, policy_path)?;
        let policy = parse_policy(&policy_source.content)
            .map_err(|error| anyhow!("policy preset parse/validation failed: {error}"))?;
        let binding_source = load_binding_source(&self.root, binding_path)?;
        let binding = parse_host_binding(&binding_source.content).map_err(anyhow::Error::msg)?;
        validate_codex_runtime(&binding, codex_probe)?;
        let now_unix = env::var("PLANR_PRESET_NOW_UNIX")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            });
        let mut composed = compose_preset(&policy, &binding, now_unix);
        if !composed.compatibility.ok {
            bail!(
                "preset compatibility failed before mutation: {}",
                composed.compatibility.errors.join("; ")
            );
        }
        let pack = validate_pack(
            &policy,
            &binding,
            policy_source.builtin.map(|source| source.id),
            binding_source.builtin.map(|source| source.id),
        );
        composed
            .compatibility
            .warnings
            .extend(pack.warnings.iter().cloned());

        let current_policy = match load_policy(&self.root) {
            PolicyLoad::Loaded(current) => Some(current),
            PolicyLoad::Missing => None,
            PolicyLoad::Invalid(error) => {
                bail!("active policy is invalid; resolve it before composition: {error}")
            }
        };
        let permission_diff = permission_additions(current_policy.as_ref(), &policy);
        let policy_content = toml::to_string_pretty(&policy)?;
        let registry_content = toml::to_string_pretty(&composed.registry)?;
        let mut artifacts = vec![
            ProposedArtifact::new(ACTIVE_POLICY_PATH, "active_policy", policy_content),
            ProposedArtifact::new(ACTIVE_REGISTRY_PATH, "agent_registry", registry_content),
        ];
        artifacts.extend(binding.artifacts.iter().map(ProposedArtifact::from_binding));
        let artifact_hashes = artifacts
            .iter()
            .map(|artifact| (artifact.path.clone(), sha256(artifact.content.as_bytes())))
            .collect::<BTreeMap<_, _>>();
        let lock = PresetLock {
            schema_version: PRESET_LOCK_SCHEMA_VERSION,
            policy: LockedSource {
                id: policy.id.clone(),
                version: policy.version.clone(),
                sha256: sha256(policy_source.content.as_bytes()),
            },
            binding: LockedSource {
                id: binding.id.clone(),
                version: binding.version.clone(),
                sha256: sha256(binding_source.content.as_bytes()),
            },
            host: binding.host.clone(),
            planr_version: env!("CARGO_PKG_VERSION").to_string(),
            verification_id: binding.verification.id.clone(),
            verification_status: composed.verification_age.status,
            applied_at: env::var("PLANR_PRESET_APPLIED_AT").unwrap_or_else(|_| now_string()),
            artifact_hashes: artifact_hashes.clone(),
            local_modifications: Vec::new(),
        };
        artifacts.push(ProposedArtifact::new(
            PRESET_LOCK_PATH,
            "preset_lock",
            toml::to_string_pretty(&lock)?,
        ));
        reject_duplicate_targets(&artifacts)?;
        let previews = preview_artifacts(&self.root, &artifacts)?;
        let installed_lock = inspect_installed_lock(&self.root, &binding)?;
        let conflicts = previews
            .iter()
            .filter(|artifact| artifact.action == ArtifactAction::Conflict)
            .map(|artifact| artifact.path.clone())
            .collect::<Vec<_>>();
        if confirm && !conflicts.is_empty() {
            if installed_lock["status"] == "fresh_apply_required" {
                bail!(
                    "installed preset lock is incompatible with the native Codex contract; no translation or legacy fallback is available. Remove the old generated preset artifacts, then run an explicit fresh preview and apply"
                );
            }
            bail!(
                "preset apply refused existing unrelated configuration: {}",
                conflicts.join(", ")
            );
        }
        let resolved_artifacts = artifacts
            .into_iter()
            .zip(&previews)
            .filter(|(_, preview)| preview.action == ArtifactAction::Create)
            .map(|(artifact, _)| ResolvedArtifact {
                target: validated_target(&self.root, &artifact.path)
                    .expect("preview already validated target"),
                content: artifact.content,
            })
            .collect::<Vec<_>>();

        let value = json!({
            "action": if confirm { "ready_to_apply" } else { "previewed" },
            "composition": composition_value(&composed),
            "permission_diff": permission_diff,
            "compatibility": composed.compatibility,
            "pack": pack,
            "verification_age": composed.verification_age,
            "provenance": lock,
            "installed_lock": installed_lock,
            "artifacts": previews,
            "conflicts": conflicts,
            "mutation": false,
        });
        Ok((
            value,
            PresetApplication {
                artifacts: resolved_artifacts,
                previews,
                permission_diff,
                lock,
            },
        ))
    }
}

fn registry_catalog_preview(
    entry: &crate::preset_registry::RegistryEntry,
    content_root: &Path,
) -> Result<Value> {
    let evaluation = entry
        .evaluation
        .as_ref()
        .ok_or_else(|| anyhow!("registry catalog projection requires evaluation metadata"))?;
    let artifact_path = |kind| {
        let mut matches = entry
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == kind);
        let artifact = matches
            .next()
            .ok_or_else(|| anyhow!("registry catalog projection requires one {kind:?} artifact"))?;
        if matches.next().is_some() {
            bail!("registry catalog projection requires one {kind:?} artifact");
        }
        Ok(content_root.join(&artifact.path))
    };
    let policy_raw = fs::read_to_string(artifact_path(
        crate::preset_registry::RegistryArtifactKind::Policy,
    )?)?;
    let binding_raw = fs::read_to_string(artifact_path(
        crate::preset_registry::RegistryArtifactKind::HostBinding,
    )?)?;
    let policy = parse_policy(&policy_raw).map_err(anyhow::Error::msg)?;
    let binding = parse_host_binding(&binding_raw).map_err(anyhow::Error::msg)?;
    let composed = compose_preset(&policy, &binding, entry.verified_at_unix);
    let pack = validate_pack(
        &policy,
        &binding,
        Some(&evaluation.policy_id),
        Some(&evaluation.binding_id),
    );
    Ok(json!({
        "pack": pack,
        "composition": composition_value(&composed),
        "artifacts": [
            {
                "kind": "active_policy",
                "config_diff": {"proposed": {"value": policy}},
            },
            {
                "kind": "agent_registry",
                "config_diff": {"proposed": {"value": composed.registry}},
            },
        ],
    }))
}

pub(crate) fn canonical_codex_registry_content() -> Result<String> {
    let policy_source = builtin_policy(Path::new("balanced")).expect("built-in balanced policy");
    let binding_source =
        builtin_binding(Path::new("codex-openai")).expect("built-in codex-openai binding");
    let policy = parse_policy(policy_source.content)
        .map_err(|error| anyhow!("built-in balanced policy is invalid: {error}"))?;
    let binding = parse_host_binding(binding_source.content).map_err(anyhow::Error::msg)?;
    let composed = compose_preset(&policy, &binding, binding.verification.verified_at_unix);
    if !composed.compatibility.ok {
        bail!(
            "built-in Codex registry does not compose: {}",
            composed.compatibility.errors.join("; ")
        );
    }
    toml::to_string_pretty(&composed.registry).map_err(Into::into)
}

fn validate_codex_runtime(
    binding: &crate::preset::HostBindingV1,
    probe: &CodexCapabilityProbe,
) -> Result<()> {
    if !binding
        .profiles
        .values()
        .any(|profile| profile.client == "codex")
    {
        return Ok(());
    }
    let output = std::process::Command::new("codex")
        .arg("--version")
        .output()
        .map_err(|error| anyhow!("Codex capability check failed: cannot execute `codex --version` ({error}); install Codex >= {MINIMUM_CODEX_VERSION} and ensure it is on PATH"))?;
    if !output.status.success() {
        bail!(
            "Codex capability check failed: `codex --version` exited with {}; upgrade or repair the installed Codex executable",
            output.status
        );
    }
    let version_output = String::from_utf8(output.stdout)
        .map_err(|_| anyhow!("Codex capability check failed: `codex --version` was not UTF-8"))?;
    let version = version_output.trim();
    let parsed = parse_version(version).ok_or_else(|| {
        anyhow!(
            "Codex capability check failed: `{version}` is not a supported semantic version; Codex >= {MINIMUM_CODEX_VERSION} is required"
        )
    })?;
    if parsed < (0, 144, 0) {
        bail!(
            "Codex capability check failed: host {version} is unsupported; upgrade to >= {MINIMUM_CODEX_VERSION}"
        );
    }
    let live_host = probe.live_host.as_ref().ok_or_else(|| anyhow!(
        "Codex capability check failed: native-v2 and active-backend evidence is missing; provide --live-host-command with a challenge-bound adapter plus --trusted-telemetry-signer and --trusted-telemetry-collector"
    ))?;
    let telemetry = probe.trusted_telemetry.as_ref().ok_or_else(|| anyhow!(
        "Codex capability check failed: unsigned host assertions are not accepted; configure .planr/trusted-telemetry.toml and provide its signer and hash-pinned collector"
    ))?;
    crate::preset_eval::verify_codex_binding_capabilities(binding, live_host, telemetry, version)
        .map_err(|error| anyhow!("Codex capability check failed: {error}"))?;
    Ok(())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let candidate = value.split_whitespace().last()?.trim_start_matches('v');
    let version = candidate
        .split_once('-')
        .map_or(candidate, |(base, _)| base);
    let mut parts = version.split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

pub(crate) fn codex_capability_probe(
    root: &Path,
    executable: Option<PathBuf>,
    args: Vec<String>,
    signer: Option<String>,
    collector: Option<PathBuf>,
) -> Result<CodexCapabilityProbe> {
    match (executable, signer, collector) {
        (None, None, None) => Ok(CodexCapabilityProbe::default()),
        (Some(executable), Some(signer_id), Some(collector)) => Ok(CodexCapabilityProbe {
            live_host: Some(crate::preset_eval::LiveHostCommand { executable, args }),
            trusted_telemetry: Some(crate::preset_eval::TrustedTelemetryCommand {
                registry: root.join(".planr/trusted-telemetry.toml"),
                signer_id,
                collector,
            }),
        }),
        _ => bail!(
            "Codex capability evidence requires --live-host-command, --trusted-telemetry-signer, and --trusted-telemetry-collector together"
        ),
    }
}

pub(crate) fn codex_capability_probe_mcp(
    root: &Path,
    args: &Value,
) -> Result<CodexCapabilityProbe> {
    let string = |field: &str| -> Result<Option<String>> {
        args.get(field)
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| anyhow!("{field} must be a string"))
            })
            .transpose()
    };
    let live_args = args
        .get("live_host_args")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| anyhow!("live_host_args must be an array"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| anyhow!("live_host_args entries must be strings"))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    codex_capability_probe(
        root,
        string("live_host_command")?.map(PathBuf::from),
        live_args,
        string("trusted_telemetry_signer")?,
        string("trusted_telemetry_collector")?.map(PathBuf::from),
    )
}

fn inspect_installed_lock(root: &Path, binding: &crate::preset::HostBindingV1) -> Result<Value> {
    let path = root.join(PRESET_LOCK_PATH);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"status": "absent"}));
        }
        Err(error) => return Err(error.into()),
    };
    let parsed = toml::from_str::<toml::Value>(&raw).ok();
    let schema = parsed
        .as_ref()
        .and_then(|value| value.get("schema_version"))
        .and_then(toml::Value::as_integer);
    let installed_binding = parsed
        .as_ref()
        .and_then(|value| value.get("binding"))
        .and_then(|value| value.get("version"))
        .and_then(toml::Value::as_str);
    let contains_codex = binding
        .profiles
        .values()
        .any(|profile| profile.client == "codex");
    if contains_codex
        && (schema != Some(PRESET_LOCK_SCHEMA_VERSION.into())
            || installed_binding != Some(binding.version.as_str()))
    {
        return Ok(json!({
            "status": "fresh_apply_required",
            "reason": "installed lock predates or differs from the native Codex contract; it is not translated or emulated",
            "schema_version": schema,
            "binding_version": installed_binding,
        }));
    }
    Ok(json!({
        "status": "current",
        "schema_version": schema,
        "binding_version": installed_binding,
    }))
}

fn load_registry_trust_store(
    repository_root: &Path,
    explicit: Option<&Path>,
) -> Result<Option<crate::preset_registry::MaintainerTrustStore>> {
    let default = repository_root.join(crate::preset_registry::TRUST_STORE_PATH);
    let path = explicit.unwrap_or(&default);
    if !path.exists() && explicit.is_none() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read maintainer trust store {}", path.display()))?;
    crate::preset_registry::parse_trust_store(&raw)
        .map(Some)
        .map_err(anyhow::Error::msg)
}

fn optional_registry_u64(value: &Value, field: &str) -> Result<Option<u64>> {
    value
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| anyhow!("{field} must be an unsigned integer"))
        })
        .transpose()
}

fn optional_registry_string(value: &Value, field: &str) -> Result<Option<String>> {
    value
        .get(field)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow!("{field} must be a string"))
        })
        .transpose()
}

fn write_evaluation_reports(
    root: &Path,
    report_dir: &Path,
    value: &Value,
    markdown: &str,
) -> Result<()> {
    let components = report_dir.components().collect::<Vec<_>>();
    if components.is_empty()
        || report_dir.is_absolute()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("preset evaluation report directory must be normalized and repository-relative");
    }
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("cannot canonicalize repository root {}", root.display()))?;
    let mut directory = canonical_root.clone();
    for component in components {
        directory.push(component.as_os_str());
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() => bail!(
                "preset evaluation report directory crosses symlink `{}`",
                directory.display()
            ),
            Ok(metadata) if !metadata.is_dir() => bail!(
                "preset evaluation report directory component is not a directory: {}",
                directory.display()
            ),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&directory).with_context(|| {
                    format!("cannot create report directory {}", directory.display())
                })?;
            }
            Err(error) => return Err(error.into()),
        }
        let canonical = fs::canonicalize(&directory)?;
        if !canonical.starts_with(&canonical_root) {
            bail!("preset evaluation report directory escapes the canonical repository root");
        }
        directory = canonical;
    }

    let machine_path = directory.join("verification.json");
    let human_path = directory.join("report.md");
    if machine_path.exists() || human_path.exists() {
        bail!(
            "preset evaluation report conflict: immutable report already exists under {}",
            directory.display()
        );
    }
    let mut machine = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&machine_path)
        .with_context(|| format!("refusing to overwrite {}", machine_path.display()))?;
    let mut human = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&human_path)
        .with_context(|| format!("refusing to overwrite {}", human_path.display()))
    {
        Ok(file) => file,
        Err(error) => {
            let _ = fs::remove_file(&machine_path);
            return Err(error);
        }
    };
    let result = (|| -> Result<()> {
        machine.write_all(&serde_json::to_vec_pretty(value)?)?;
        human.write_all(markdown.as_bytes())?;
        machine.sync_all()?;
        human.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&machine_path);
        let _ = fs::remove_file(&human_path);
    }
    result
}

fn composition_value(composed: &ComposedPreset) -> Value {
    json!({
        "policy": {"id": composed.policy_id, "version": composed.policy_version},
        "binding": {"id": composed.binding_id, "version": composed.binding_version},
        "host": composed.host,
        "profiles": composed.profiles,
        "dispatch": composed.dispatch,
    })
}

fn resolve_input(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

struct LoadedPresetSource {
    content: String,
    builtin: Option<BuiltinSource>,
}

fn load_policy_source(root: &Path, path: &Path) -> Result<LoadedPresetSource> {
    load_source(root, path, "policy preset", builtin_policy(path))
}

fn load_binding_source(root: &Path, path: &Path) -> Result<LoadedPresetSource> {
    load_source(root, path, "host binding", builtin_binding(path))
}

fn load_source(
    root: &Path,
    path: &Path,
    kind: &str,
    builtin: Option<BuiltinSource>,
) -> Result<LoadedPresetSource> {
    let resolved = resolve_input(root, path);
    match fs::read_to_string(&resolved) {
        Ok(content) => Ok(LoadedPresetSource {
            content,
            builtin: None,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let source =
                builtin.with_context(|| format!("failed to read {kind} {}", path.display()))?;
            Ok(LoadedPresetSource {
                content: source.content.to_string(),
                builtin: Some(source),
            })
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to read {kind} {}", path.display()))
        }
    }
}

#[derive(Clone)]
struct ProposedArtifact {
    path: String,
    kind: String,
    content: String,
}

impl ProposedArtifact {
    fn new(path: impl Into<String>, kind: impl Into<String>, content: String) -> Self {
        Self {
            path: path.into(),
            kind: kind.into(),
            content,
        }
    }

    fn from_binding(artifact: &BindingArtifact) -> Self {
        Self::new(&artifact.path, &artifact.kind, artifact.content.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactAction {
    Create,
    Unchanged,
    Conflict,
}

#[derive(Clone, Debug, Serialize)]
struct ArtifactPreview {
    path: String,
    kind: String,
    action: ArtifactAction,
    proposed_sha256: String,
    existing_sha256: Option<String>,
    bytes: usize,
    config_diff: ArtifactConfigDiff,
}

#[derive(Clone, Debug, Serialize)]
struct ArtifactConfigDiff {
    current: Option<Value>,
    proposed: Value,
}

struct ResolvedArtifact {
    target: PathBuf,
    content: String,
}

struct PresetApplication {
    artifacts: Vec<ResolvedArtifact>,
    previews: Vec<ArtifactPreview>,
    permission_diff: BTreeMap<String, crate::preset::RolePermissionAdditions>,
    lock: PresetLock,
}

fn reject_duplicate_targets(artifacts: &[ProposedArtifact]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for artifact in artifacts {
        if !seen.insert(&artifact.path) {
            bail!("preset declares duplicate target `{}`", artifact.path);
        }
    }
    Ok(())
}

fn preview_artifacts(root: &Path, artifacts: &[ProposedArtifact]) -> Result<Vec<ArtifactPreview>> {
    artifacts
        .iter()
        .map(|artifact| {
            let target = validated_target(root, &artifact.path)?;
            let proposed_hash = sha256(artifact.content.as_bytes());
            let existing = if target.exists() {
                Some(fs::read(&target).with_context(|| {
                    format!("failed to inspect existing target {}", target.display())
                })?)
            } else {
                None
            };
            let existing_hash = existing.as_deref().map(sha256);
            let config_diff = ArtifactConfigDiff {
                current: existing.as_deref().map(config_representation),
                proposed: config_representation(artifact.content.as_bytes()),
            };
            let action = match existing.as_deref() {
                None => ArtifactAction::Create,
                Some(existing) if existing == artifact.content.as_bytes() => {
                    ArtifactAction::Unchanged
                }
                Some(_) => ArtifactAction::Conflict,
            };
            Ok(ArtifactPreview {
                path: artifact.path.clone(),
                kind: artifact.kind.clone(),
                action,
                proposed_sha256: proposed_hash,
                existing_sha256: existing_hash,
                bytes: artifact.content.len(),
                config_diff,
            })
        })
        .collect()
}

fn config_representation(content: &[u8]) -> Value {
    let Ok(text) = std::str::from_utf8(content) else {
        return json!({
            "format": "binary",
            "bytes": content.len(),
            "sha256": sha256(content),
        });
    };
    if let Ok(value) = toml::from_str::<toml::Value>(text)
        && let Ok(mut value) = serde_json::to_value(value)
    {
        redact_value(None, &mut value);
        return json!({"format": "toml", "value": value});
    }
    let lines = text
        .lines()
        .map(|line| {
            let key = line
                .split_once('=')
                .or_else(|| line.split_once(':'))
                .map(|(key, _)| key.trim());
            if key.is_some_and(sensitive_key) || sensitive_value(line) {
                "[REDACTED]".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    json!({"format": "text", "lines": lines})
}

fn redact_value(key: Option<&str>, value: &mut Value) {
    if key.is_some_and(sensitive_key) {
        *value = json!("[REDACTED]");
        return;
    }
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                redact_value(Some(key), value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_value(None, value);
            }
        }
        Value::String(text) if sensitive_value(text) => *text = "[REDACTED]".to_string(),
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("secret")
        || key.contains("password")
        || key.contains("credential")
        || key == "token"
        || key.ends_with("_token")
        || key == "key"
        || key.ends_with("_key")
}

fn sensitive_value(value: &str) -> bool {
    looks_secret_like(value)
}

fn validated_target(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("cannot canonicalize repository root {}", root.display()))?;
    let path = Path::new(relative);
    if path.is_absolute() {
        bail!("preset target `{relative}` must be repository-relative");
    }
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow!("preset target `{relative}` is not valid UTF-8")),
            _ => Err(anyhow!(
                "preset target `{relative}` contains absolute, current, or parent traversal"
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    let normalized = components.join("/");
    if normalized != relative {
        bail!("preset target `{relative}` is not normalized as `{normalized}`");
    }
    if !allowed_repository_target(&normalized) {
        bail!(
            "preset target `{relative}` is outside the repository artifact allowlist; user/global config and .codex/config.toml are forbidden"
        );
    }

    let mut cursor = root.clone();
    for component in &components {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "preset target `{relative}` crosses symlink `{}`",
                    cursor.display()
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if !cursor.starts_with(&root) {
        bail!("preset target `{relative}` escapes the canonical repository root");
    }
    Ok(cursor)
}

fn allowed_repository_target(path: &str) -> bool {
    matches!(
        path,
        ACTIVE_POLICY_PATH | ACTIVE_REGISTRY_PATH | PRESET_LOCK_PATH
    ) || [
        ".codex/agents/",
        ".codex/skills/",
        ".claude/agents/",
        ".claude/skills/",
        ".cursor/agents/",
        ".cursor/skills/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix) && path.len() > prefix.len())
}

fn apply_artifacts(artifacts: &[ResolvedArtifact]) -> Result<()> {
    let mut created = Vec::new();
    for artifact in artifacts {
        let result = (|| -> Result<()> {
            if let Some(parent) = artifact.target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&artifact.target)
                .with_context(|| {
                    format!(
                        "target appeared after preview; refusing overwrite: {}",
                        artifact.target.display()
                    )
                })?;
            file.write_all(artifact.content.as_bytes())?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&artifact.target);
            for path in created.iter().rev() {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        created.push(artifact.target.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn repository_writer_rejects_escapes_and_codex_config() {
        let root = tempdir().unwrap();
        for path in [
            "/tmp/outside",
            "../outside",
            ".codex/config.toml",
            ".config/codex/config.toml",
            ".codex/agents/../config.toml",
        ] {
            assert!(
                validated_target(root.path(), path).is_err(),
                "accepted {path}"
            );
        }
        assert!(validated_target(root.path(), ".codex/agents/worker.toml").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn repository_writer_rejects_symlink_escape_before_write() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".codex")).unwrap();
        symlink(outside.path(), root.path().join(".codex/agents")).unwrap();
        assert!(validated_target(root.path(), ".codex/agents/worker.toml").is_err());
        assert!(!outside.path().join("worker.toml").exists());
    }

    #[test]
    fn evaluation_writer_is_repository_relative_and_immutable() {
        let root = tempdir().unwrap();
        let value = json!({"report": {"schema_version": 1}});
        for path in ["../outside", "/tmp/outside", "reports/../outside"] {
            assert!(
                write_evaluation_reports(root.path(), Path::new(path), &value, "report").is_err(),
                "accepted {path}"
            );
        }

        let report_dir = Path::new("reports/evaluation");
        write_evaluation_reports(root.path(), report_dir, &value, "report").unwrap();
        let machine = root.path().join(report_dir).join("verification.json");
        let human = root.path().join(report_dir).join("report.md");
        assert_eq!(fs::read_to_string(&human).unwrap(), "report");
        let original = fs::read(&machine).unwrap();
        let error = write_evaluation_reports(root.path(), report_dir, &value, "changed")
            .unwrap_err()
            .to_string();
        assert!(error.contains("immutable report already exists"));
        assert_eq!(fs::read(machine).unwrap(), original);
        assert_eq!(fs::read_to_string(human).unwrap(), "report");
    }

    #[cfg(unix)]
    #[test]
    fn evaluation_writer_rejects_symlink_escape_before_write() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir(root.path().join("reports")).unwrap();
        symlink(outside.path(), root.path().join("reports/escape")).unwrap();
        let error = write_evaluation_reports(
            root.path(),
            Path::new("reports/escape"),
            &json!({"report": {}}),
            "report",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("crosses symlink"));
        assert!(!outside.path().join("verification.json").exists());
        assert!(!outside.path().join("report.md").exists());
    }

    #[test]
    fn config_diff_redacts_secret_like_toml_and_text() {
        let toml = config_representation(
            br#"model = "gpt-5.5"
api_key = "sk-do-not-print"
aws_access_key_id = "AKIAEXAMPLE123"
notes = "rotate the credential xoxb-embedded-token today"
"#,
        );
        assert_eq!(toml["value"]["model"], "gpt-5.5");
        assert_eq!(toml["value"]["api_key"], "[REDACTED]");
        assert_eq!(toml["value"]["aws_access_key_id"], "[REDACTED]");
        assert_eq!(toml["value"]["notes"], "[REDACTED]");
        assert!(!toml.to_string().contains("sk-do-not-print"));
        assert!(!toml.to_string().contains("AKIAEXAMPLE123"));
        assert!(!toml.to_string().contains("xoxb-embedded-token"));

        let text = config_representation(
            b"model: gpt-5.5\naws_access_key_id: AKIAEXAMPLE123\nnotes: rotate xoxb-embedded-token today\n",
        );
        assert_eq!(text["lines"][0], "model: gpt-5.5");
        assert_eq!(text["lines"][1], "[REDACTED]");
        assert_eq!(text["lines"][2], "[REDACTED]");
        assert!(!text.to_string().contains("AKIAEXAMPLE123"));
        assert!(!text.to_string().contains("xoxb-embedded-token"));
    }
}
