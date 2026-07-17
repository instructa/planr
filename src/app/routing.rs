//! Provider-neutral routing bundle application boundary.

use super::App;
use crate::cli::{RoutingBundleCommand, RoutingCommand};
use crate::routing_bundle::{
    RoutingApplicationRecordV1, RoutingBundleTrustAnchorV1, RoutingBundleV1, apply_bundle,
    load_bundle, preview_bundle,
};
use crate::util::now_string;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;

impl App {
    pub(crate) fn routing(&self, command: RoutingCommand) -> Result<()> {
        match command {
            RoutingCommand::Bundle(args) => self.routing_bundle(args.command),
        }
    }

    fn routing_bundle(&self, command: RoutingBundleCommand) -> Result<()> {
        match command {
            RoutingBundleCommand::Inspect(args) => {
                let trust = trust_anchor(&args)?;
                let bundle = load_bundle(&args.bundle, trust.as_ref())?;
                let value = inspect_value(&bundle)?;
                self.emit(
                    value,
                    format!(
                        "routing bundle {} is valid ({} artifact(s), {} profile(s), {} route(s))",
                        bundle.bundle_id,
                        bundle.artifacts.len(),
                        bundle.profiles.len(),
                        bundle.routes.len()
                    ),
                )
            }
            RoutingBundleCommand::Preview(args) => {
                let trust = trust_anchor(&args)?;
                let bundle = load_bundle(&args.bundle, trust.as_ref())?;
                let preview = preview_bundle(&self.root, &bundle, trust.as_ref())?;
                self.record_event(
                    "routing.bundle.previewed",
                    None,
                    serde_json::to_value(&preview)?,
                )?;
                let applicable = preview.is_applicable();
                let human = format!(
                    "previewed routing bundle {}: {} artifact(s), {} conflict(s)",
                    preview.bundle_id,
                    preview.artifacts.len(),
                    preview.conflicts.len()
                );
                self.emit(json!({"applicable": applicable, "preview": preview}), human)
            }
            RoutingBundleCommand::Apply(args) => {
                let trust = trust_anchor(&args)?;
                let bundle = match load_bundle(&args.bundle, trust.as_ref()) {
                    Ok(bundle) => bundle,
                    Err(error) => {
                        let _ = self.record_event(
                            "routing.bundle.rejected",
                            None,
                            json!({"reason": error.to_string()}),
                        );
                        return Err(error);
                    }
                };
                let (preview, receipts) = match apply_bundle(&self.root, &bundle, trust.as_ref()) {
                    Ok(result) => result,
                    Err(error) => {
                        let _ = self.record_event(
                            "routing.bundle.rejected",
                            None,
                            json!({
                                "bundle_id": bundle.bundle_id,
                                "reason": error.to_string()
                            }),
                        );
                        return Err(error);
                    }
                };
                let record = RoutingApplicationRecordV1 {
                    schema_version: 1,
                    bundle_id: bundle.bundle_id.clone(),
                    bundle_sha256: preview.bundle_sha256.clone(),
                    repository_id: repository_id(&self.root)?,
                    previewed_at: now_string(),
                    applied_at: Some(now_string()),
                    artifacts: receipts,
                    conflicts: Vec::new(),
                    declared_routes: bundle
                        .routes
                        .iter()
                        .map(
                            |route| crate::routing_bundle::RoutingDeclaredRouteEvidenceV1 {
                                selector: route_selector(route),
                                profile: route.profile.clone(),
                            },
                        )
                        .chain(bundle.route_default.iter().map(|route| {
                            crate::routing_bundle::RoutingDeclaredRouteEvidenceV1 {
                                selector: "default".to_string(),
                                profile: route.profile.clone(),
                            }
                        }))
                        .collect(),
                    effective_routes: Vec::new(),
                };
                record.validate().map_err(anyhow::Error::new)?;
                self.record_event(
                    "routing.bundle.applied",
                    None,
                    serde_json::to_value(&record)?,
                )?;
                let changed = record
                    .artifacts
                    .iter()
                    .filter(|artifact| {
                        !matches!(
                            artifact.outcome,
                            crate::routing_bundle::RoutingArtifactOutcomeV1::Unchanged
                        )
                    })
                    .count();
                self.emit(
                    json!({"application": record}),
                    format!(
                        "applied routing bundle {}: {} changed artifact(s)",
                        bundle.bundle_id, changed
                    ),
                )
            }
        }
    }
}

fn inspect_value(bundle: &RoutingBundleV1) -> Result<Value> {
    Ok(json!({
        "valid": true,
        "bundle": bundle,
        "bundle_sha256": bundle.digest().map_err(anyhow::Error::new)?,
    }))
}

fn trust_anchor(
    args: &crate::cli::RoutingBundleFileArgs,
) -> Result<Option<RoutingBundleTrustAnchorV1>> {
    match (&args.trusted_signer, &args.trusted_public_key_file) {
        (None, None) => Ok(None),
        (Some(signer), Some(path)) => Ok(Some(RoutingBundleTrustAnchorV1 {
            signer: signer.clone(),
            public_key: fs::read_to_string(path)?.trim().to_string(),
        })),
        _ => bail!("--trusted-signer and --trusted-public-key-file must be supplied together"),
    }
}

fn repository_id(root: &std::path::Path) -> Result<String> {
    let canonical = fs::canonicalize(root)?;
    Ok(format!(
        "sha256:{}",
        hex_digest(canonical.to_string_lossy().as_bytes())
    ))
}

fn route_selector(route: &crate::agents::Route) -> String {
    let mut parts = Vec::new();
    if let Some(work_type) = &route.selector.work_type {
        parts.push(format!("work_type={work_type}"));
    }
    if let Some(plan) = &route.selector.plan {
        parts.push(format!("plan={plan}"));
    }
    parts.join(",")
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
