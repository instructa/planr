#![allow(dead_code)]

use super::host::{HostCaptureEvaluation, evaluate_phase1_host_fixture};
use crate::canonical_json::sha256_json_digest;
use crate::evidence::model::{
    NamespacedIdentifier, SchemaVersion, Sha256Digest, VerificationCapabilityManifest,
};
use crate::evidence::registry::CapabilityRegistry;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) const CODEX_CHROME_BROWSER_CLIENT_SURFACE: &str = "chrome-browser-client";

#[derive(Debug, Clone)]
pub(crate) struct CodexCaptureAdapter {
    pub experiment_id: String,
    pub host: String,
    pub surface: String,
    pub trusted_adapter_enabled: bool,
    pub availability_status: String,
    pub matrix_entry: Value,
    pub product_boundary: String,
    pub reason: String,
    pub manifest: Option<VerificationCapabilityManifest>,
    pub instance: Option<Value>,
}

pub(crate) fn evaluate_codex_phase1_fixture(root: &Path) -> Result<Vec<CodexCaptureAdapter>> {
    evaluate_phase1_host_fixture(root)?
        .into_iter()
        .map(classify_codex_capture)
        .collect()
}

pub(crate) fn evaluate_codex_phase1_embedded() -> Result<Vec<CodexCaptureAdapter>> {
    let fixture = materialize_embedded_phase1_fixture()?;
    evaluate_codex_phase1_fixture(fixture.path())
}

pub(crate) fn store_embedded_verified_codex_host_capabilities(conn: &Connection) -> Result<()> {
    let adapters = evaluate_codex_phase1_embedded()?;
    for adapter in adapters
        .into_iter()
        .filter(|adapter| adapter.trusted_adapter_enabled)
    {
        let manifest = adapter
            .manifest
            .context("enabled Codex host adapter missing manifest")?;
        let instance_value = adapter
            .instance
            .context("enabled Codex host adapter missing instance")?;
        let instance = serde_json::from_value(instance_value)
            .context("enabled Codex host adapter instance is not canonical")?;
        let mut registry =
            CapabilityRegistry::from_manifests_and_adapter_registrations(Path::new("."), [], &[]);
        registry.store_verified_host_capture_instance(conn, manifest, instance)?;
    }
    Ok(())
}

pub(crate) fn host_adapter_classifications_value() -> Value {
    let adapters = match evaluate_codex_phase1_embedded() {
        Ok(adapters) => adapters,
        Err(error) => {
            return json!({
                "schema_version": "planr.host_adapter_classifications.v1",
                "host": "codex",
                "phase1_fixture_contract": "host-capability-raw/1.0.0",
                "surfaces": [],
                "validation_error": {
                    "reason": "embedded Phase-1 host capability contract failed validation",
                    "detail": error.to_string()
                },
                "mechanical_rule": "Adapter availability is derived from the validated embedded Phase-1 contract and fails closed on drift."
            });
        }
    };
    let surfaces = adapters
        .iter()
        .map(|adapter| {
            json!({
                "host": adapter.host,
                "surface": adapter.surface,
                "trusted_adapter_enabled": adapter.trusted_adapter_enabled,
                "availability_status": adapter.availability_status,
                "reason": adapter.reason,
                "host_version": adapter.matrix_entry["host_version"],
                "observation_types": adapter.matrix_entry["observation_types"],
                "provenance": adapter.matrix_entry["provenance"],
                "permissions": adapter.matrix_entry["permissions"],
                "artifact_kinds": adapter.matrix_entry["artifact_kinds"],
                "blind_spots": adapter.matrix_entry["blind_spots"],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": "planr.host_adapter_classifications.v1",
        "host": "codex",
        "phase1_fixture_contract": "host-capability-raw/1.0.0",
        "surfaces": surfaces,
        "mechanical_rule": "Only available observed_capture payloads whose surface is a production Codex capture/import boundary are enabled. Harness probes, degraded captures, unavailable placeholders, permission_denied captures, and unknown payload versions keep trusted_adapter_enabled=false without blocking unrelated methods."
    })
}

fn classify_codex_capture(capture: HostCaptureEvaluation) -> Result<CodexCaptureAdapter> {
    if capture.host != "codex" {
        return Ok(disabled(
            capture,
            "non-Codex host capture cannot enable a Codex adapter",
        ));
    }

    if capture.final_status.as_str() != "available" {
        let reason = format!(
            "{}; adapter disabled with canonical availability status {}",
            capture.availability_reason,
            capture.final_status.as_str()
        );
        return Ok(disabled(capture, reason));
    }

    if capture.claim_source != "observed_capture"
        || capture.source_kind != "external_observed_capture"
        || capture.observation_mode != "observed_payload"
    {
        return Ok(disabled(
            capture,
            "available status is not backed by external observed capture provenance",
        ));
    }

    match capture.surface.as_str() {
        "terminal" => Ok(disabled(
            capture,
            "harness-only terminal probe; functions.exec_command is not a production Codex capture/import adapter boundary",
        )),
        "local-function" => Ok(disabled(
            capture,
            "harness-only MCP Node REPL probe; mcp__node_repl__js is not a production Codex capture/import adapter boundary",
        )),
        CODEX_CHROME_BROWSER_CLIENT_SURFACE => enable_chrome_browser_client(capture),
        _ => Ok(disabled(
            capture,
            "available capture has no enabled Codex adapter policy",
        )),
    }
}

pub(crate) fn enable_chrome_browser_client(
    capture: HostCaptureEvaluation,
) -> Result<CodexCaptureAdapter> {
    enable_chrome_browser_client_at_trust_boundary(
        capture,
        ChromeBrowserClientTrustBoundary::UntrustedExternal,
    )
}

pub(crate) fn enable_chrome_browser_client_from_verifier_admission(
    capture: HostCaptureEvaluation,
) -> Result<CodexCaptureAdapter> {
    enable_chrome_browser_client_at_trust_boundary(
        capture,
        ChromeBrowserClientTrustBoundary::VerifierAdmission,
    )
}

#[derive(Clone, Copy)]
enum ChromeBrowserClientTrustBoundary {
    UntrustedExternal,
    VerifierAdmission,
}

fn enable_chrome_browser_client_at_trust_boundary(
    capture: HostCaptureEvaluation,
    trust_boundary: ChromeBrowserClientTrustBoundary,
) -> Result<CodexCaptureAdapter> {
    if capture.final_status.as_str() != "available" {
        return Ok(disabled(
            capture,
            "Chrome browser-client capture did not finish with available status",
        ));
    }
    if capture.tool_name != "browser-client.mjs chrome Runtime.evaluate"
        || capture.event_source != "browser-client:chrome"
        || capture.input_kind != "controlled_probe"
        || capture.instance.host_version == "missing"
        || capture
            .missing_fields
            .iter()
            .any(|field| field == "host_version")
        || matches!(
            trust_boundary,
            ChromeBrowserClientTrustBoundary::UntrustedExternal
        )
        || !capture
            .artifact_refs
            .iter()
            .any(|artifact| artifact.kind == "cdp-json-result")
    {
        return Ok(disabled(
            capture,
            "Chrome browser-client capture is not backed by a trusted Planr execution or verifier-admission boundary with required host version, CDP final event, and artifact binding",
        ));
    }

    let manifest = chrome_browser_client_manifest(&capture)?;
    let bound_instance_value = bind_instance_to_manifest(&capture, &manifest)?;
    let matrix_entry = host_surface_matrix_entry(&capture);
    Ok(CodexCaptureAdapter {
        experiment_id: capture.experiment_id,
        host: capture.host,
        surface: capture.surface,
        trusted_adapter_enabled: true,
        availability_status: "available".to_string(),
        matrix_entry,
        product_boundary: "browser-client:chrome Runtime.evaluate + DOM observation".to_string(),
        reason: "native Chrome browser-client produced a final CDP Runtime.evaluate/DOM observation with content-bound artifact and provenance".to_string(),
        manifest: Some(manifest),
        instance: Some(bound_instance_value),
    })
}

fn disabled(capture: HostCaptureEvaluation, reason: impl Into<String>) -> CodexCaptureAdapter {
    CodexCaptureAdapter {
        matrix_entry: host_surface_matrix_entry(&capture),
        experiment_id: capture.experiment_id,
        host: capture.host,
        surface: capture.surface,
        trusted_adapter_enabled: false,
        availability_status: capture.final_status.as_str().to_string(),
        product_boundary: "none".to_string(),
        reason: reason.into(),
        manifest: None,
        instance: None,
    }
}

fn host_surface_matrix_entry(capture: &HostCaptureEvaluation) -> Value {
    let mut artifact_kinds = capture
        .artifact_refs
        .iter()
        .map(|artifact| artifact.kind.as_str())
        .collect::<Vec<_>>();
    artifact_kinds.sort_unstable();
    json!({
        "host_version": capture.instance.host_version.as_str(),
        "observation_types": capture.instance.observed_payload_contract.observation_types,
        "provenance": {
            "claim_source": capture.claim_source,
            "source_kind": capture.source_kind,
            "observation_mode": capture.observation_mode,
        },
        "permissions": capture.instance.permissions,
        "artifact_kinds": artifact_kinds,
        "blind_spots": capture.instance.limitations,
    })
}

fn chrome_browser_client_manifest(
    capture: &HostCaptureEvaluation,
) -> Result<VerificationCapabilityManifest> {
    let observation_type = observation_type(capture)?;
    let schema_digest = Sha256Digest::parse(capture.raw_schema_digest.clone())
        .map_err(|error| anyhow::anyhow!(error))?;
    let adapter_digest = Sha256Digest::parse(capture.raw_schema_digest.clone())
        .map_err(|error| anyhow::anyhow!(error))?;
    let value = json!({
        "id": "host-chrome-browser-client-manifest",
        "schema_version": SchemaVersion::v1().as_str(),
        "version": "1.0.0",
        "adapter_kind": "host",
        "adapter_digest": adapter_digest.as_str(),
        "supported_surfaces": [CODEX_CHROME_BROWSER_CLIENT_SURFACE],
        "supported_observations": [{
            "type": observation_type.as_str(),
            "schema_ref": "schemas/host-capability-observed-raw.schema.json",
            "schema_digest": schema_digest.as_str()
        }],
        "supported_interactions": ["navigate", "runtime.evaluate", "dom.visible-text"],
        "supported_artifacts": ["cdp-json-result"],
        "runtime_targets": [{
            "kind": "codex-host-surface",
            "id": capture.instance.environment.id.as_str(),
            "surface": CODEX_CHROME_BROWSER_CLIENT_SURFACE
        }],
        "provenance_path": "verified_host_event",
        "permissions": capture.instance.permissions,
        "costs": {"network": "external-http-target", "browser": "user-chrome-session"},
        "determinism": "target-page-dependent",
        "repeatability": "replay-validates-captured-contract",
        "independence": "host-capture-artifact-and-provenance-bound",
        "blind_spots": capture.instance.limitations,
        "availability_probe": {
            "kind": "process",
            "execution": {
                "kind": "process",
                "executable": "planr",
                "args": ["evidence", "classifications"],
                "working_directory": ".",
                "timeout_ms": 30000,
                "stdout_limit_bytes": 1048576,
                "stderr_limit_bytes": 1048576,
                "payload_schema": {
                    "type": observation_type.as_str(),
                    "schema_ref": "schemas/host-capability-observed-raw.schema.json",
                    "schema_digest": schema_digest.as_str()
                }
            }
        }
    });
    serde_json::from_value(value).context("building Chrome browser-client manifest")
}

fn bind_instance_to_manifest(capture: &HostCaptureEvaluation, manifest: &VerificationCapabilityManifest) -> Result<Value> {
    let manifest_value = serde_json::to_value(manifest).context("serializing host manifest")?;
    let manifest_digest = sha256_json_digest(&manifest_value)?;
    let mut instance_value = capture.instance_value.clone();
    instance_value["manifest_id"] = json!("host-chrome-browser-client-manifest");
    instance_value["manifest_digest"] = json!(manifest_digest);
    instance_value["adapter_version"] = json!(manifest.version);
    let instance = serde_json::from_value::<
        crate::evidence::model::VerificationCapabilityInstance,
    >(instance_value)
    .context("binding Chrome browser-client instance to manifest")?;
    serde_json::to_value(&instance).context("serializing canonical Chrome browser-client instance")
}

struct EmbeddedPhase1Fixture {
    root: PathBuf,
}

impl EmbeddedPhase1Fixture {
    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for EmbeddedPhase1Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn materialize_embedded_phase1_fixture() -> Result<EmbeddedPhase1Fixture> {
    let root = std::env::temp_dir().join(format!("planr-host-capability-{}", Uuid::new_v4()));
    for (relative, bytes) in EMBEDDED_PHASE1_FILES {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("creating embedded fixture directory {}", parent.display())
            })?;
        }
        fs::write(&path, bytes)
            .with_context(|| format!("writing embedded fixture {}", path.display()))?;
    }
    Ok(EmbeddedPhase1Fixture { root })
}

const EMBEDDED_PHASE1_FILES: &[(&str, &[u8])] = &[
    (
        "experiment-suite.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/experiment-suite.json"
        ),
    ),
    (
        "expected/normalized-manifest.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/expected/normalized-manifest.json"
        ),
    ),
    (
        "provenance/host-capability-captures.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/provenance/host-capability-captures.json"
        ),
    ),
    (
        "manifests/phase1-host-capability-manifests.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/manifests/phase1-host-capability-manifests.json"
        ),
    ),
    (
        "schemas/host-capability-observed-raw.schema.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/schemas/host-capability-observed-raw.schema.json"
        ),
    ),
    (
        "schemas/host-capability-expected-manifest.schema.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/schemas/host-capability-expected-manifest.schema.json"
        ),
    ),
    (
        "schemas/host-capability-provenance.schema.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/schemas/host-capability-provenance.schema.json"
        ),
    ),
    (
        "observed/exp-terminal.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-terminal.json"
        ),
    ),
    (
        "observed/exp-local-function.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-local-function.json"
        ),
    ),
    (
        "observed/exp-codex-app-server.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-codex-app-server.json"
        ),
    ),
    (
        "observed/exp-codex-exec.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-codex-exec.json"
        ),
    ),
    (
        "observed/exp-codex-mcp-server.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-codex-mcp-server.json"
        ),
    ),
    (
        "observed/exp-mcp-browser.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-mcp-browser.json"
        ),
    ),
    (
        "observed/exp-built-in-browser.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-built-in-browser.json"
        ),
    ),
    (
        "observed/exp-chrome-cdp.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-chrome-cdp.json"
        ),
    ),
    (
        "observed/exp-chrome-browser-client.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-chrome-browser-client.json"
        ),
    ),
    (
        "observed/exp-codex-hook-events.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-codex-hook-events.json"
        ),
    ),
    (
        "observed/exp-computer-use.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-computer-use.json"
        ),
    ),
    (
        "observed/exp-claude-code-host-capture.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-claude-code-host-capture.json"
        ),
    ),
    (
        "observed/exp-cursor-agent-host-capture.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-cursor-agent-host-capture.json"
        ),
    ),
    (
        "observed/exp-pi-cli-host-capture.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/observed/exp-pi-cli-host-capture.json"
        ),
    ),
    (
        "artifacts/terminal/stdout.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/terminal/stdout.txt"
        ),
    ),
    (
        "artifacts/claude-code-host-capture/stdout.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/claude-code-host-capture/stdout.txt"
        ),
    ),
    (
        "artifacts/claude-code-host-capture/stderr.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/claude-code-host-capture/stderr.txt"
        ),
    ),
    (
        "artifacts/claude-code-host-capture/result.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/claude-code-host-capture/result.json"
        ),
    ),
    (
        "artifacts/cursor-agent-host-capture/stdout.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/cursor-agent-host-capture/stdout.txt"
        ),
    ),
    (
        "artifacts/cursor-agent-host-capture/stderr.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/cursor-agent-host-capture/stderr.txt"
        ),
    ),
    (
        "artifacts/cursor-agent-host-capture/result.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/cursor-agent-host-capture/result.json"
        ),
    ),
    (
        "artifacts/pi-cli-host-capture/stdout.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/pi-cli-host-capture/stdout.txt"
        ),
    ),
    (
        "artifacts/pi-cli-host-capture/stderr.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/pi-cli-host-capture/stderr.txt"
        ),
    ),
    (
        "artifacts/pi-cli-host-capture/result.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/pi-cli-host-capture/result.json"
        ),
    ),
    (
        "artifacts/mcp-node-repl/result.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/mcp-node-repl/result.json"
        ),
    ),
    (
        "artifacts/mcp-playwright/tabs.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/mcp-playwright/tabs.txt"
        ),
    ),
    (
        "artifacts/built-in-browser/error.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/built-in-browser/error.txt"
        ),
    ),
    (
        "artifacts/chrome-devtools/error.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/chrome-devtools/error.txt"
        ),
    ),
    (
        "artifacts/chrome-browser-client/cdp-result.json",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/chrome-browser-client/cdp-result.json"
        ),
    ),
    (
        "artifacts/computer-use/error.txt",
        include_bytes!(
            "../../../../tests/fixtures/evidence/host-capabilities/v1/artifacts/computer-use/error.txt"
        ),
    ),
];

fn observation_type(capture: &HostCaptureEvaluation) -> Result<NamespacedIdentifier> {
    capture
        .instance
        .observed_payload_contract
        .observation_types
        .first()
        .cloned()
        .context("host capability instance has no observation types")
}

pub(crate) fn default_phase1_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evidence/host-capabilities/v1")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_adapter_enables_only_product_chrome_browser_client_capture() {
        let adapters = evaluate_codex_phase1_fixture(&default_phase1_fixture_root()).unwrap();
        assert_eq!(adapters.len(), 14);
        let enabled = adapters
            .iter()
            .filter(|adapter| adapter.trusted_adapter_enabled)
            .collect::<Vec<_>>();
        assert!(enabled.is_empty());
        let chrome_adapter = adapters
            .iter()
            .find(|adapter| adapter.experiment_id == "exp-chrome-browser-client")
            .unwrap();
        assert!(!chrome_adapter.trusted_adapter_enabled);
        assert!(chrome_adapter.reason.contains("host version"));
        for experiment_id in [
            "exp-claude-code-host-capture",
            "exp-cursor-agent-host-capture",
            "exp-pi-cli-host-capture",
        ] {
            let peer_adapter = adapters
                .iter()
                .find(|adapter| adapter.experiment_id == experiment_id)
                .unwrap();
            assert!(!peer_adapter.trusted_adapter_enabled);
            assert!(peer_adapter.reason.contains("non-Codex host capture"));
        }
        let chrome_capture = super::evaluate_phase1_host_fixture(&default_phase1_fixture_root())
            .unwrap()
            .into_iter()
            .find(|capture| capture.surface == CODEX_CHROME_BROWSER_CLIENT_SURFACE)
            .unwrap();
        assert!(
            chrome_capture
                .missing_fields
                .contains(&"screenshot_artifact".to_string())
        );
        assert!(
            chrome_capture
                .missing_fields
                .contains(&"host_version".to_string())
        );
        assert!(chrome_adapter.manifest.is_none());

        for surface in ["terminal", "local-function"] {
            let adapter = adapters
                .iter()
                .find(|adapter| adapter.surface == surface)
                .unwrap();
            assert!(!adapter.trusted_adapter_enabled);
            assert!(adapter.reason.contains("harness-only"));
            assert!(adapter.manifest.is_none());
        }

        for surface in [
            "codex-app-server",
            "codex-exec",
            "codex-mcp-server",
            "mcp-browser",
            "built-in-browser",
            "chrome-cdp",
            "codex-hook-events",
            "computer-use",
        ] {
            let adapter = adapters
                .iter()
                .find(|adapter| adapter.surface == surface)
                .unwrap();
            assert!(!adapter.trusted_adapter_enabled);
        }

        let classifications = host_adapter_classifications_value();
        assert!(
            classifications["surfaces"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["trusted_adapter_enabled"] == false)
        );
        let surfaces = classifications["surfaces"].as_array().unwrap();
        let degraded = surfaces
            .iter()
            .find(|entry| entry["surface"] == "mcp-browser")
            .unwrap();
        assert_eq!(degraded["availability_status"], "degraded");
        assert_eq!(degraded["provenance"]["claim_source"], "observed_capture");
        assert_eq!(degraded["permissions"]["network"], "not_probed");
        assert!(
            degraded["blind_spots"]
                .as_array()
                .unwrap()
                .iter()
                .any(|blind_spot| blind_spot.as_str().unwrap().contains("Playwright"))
        );
        for (host, surface) in [
            ("claude", "claude-code"),
            ("cursor", "cursor-agent"),
            ("pi", "pi-cli"),
        ] {
            let peer = surfaces
                .iter()
                .find(|entry| entry["host"] == host && entry["surface"] == surface)
                .unwrap();
            assert_eq!(peer["availability_status"], "unavailable");
            assert_eq!(peer["trusted_adapter_enabled"], false);
            assert_eq!(
                peer["provenance"]["observation_mode"],
                "mechanical_invocation"
            );
            assert!(
                peer["artifact_kinds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|kind| kind == "invocation-stdout")
            );
            assert!(peer.get("status").is_none());
        }
    }

    #[test]
    fn codex_adapter_embedded_contract_rejects_drift_before_enablement() {
        let unknown_major = materialize_embedded_phase1_fixture().unwrap();
        mutate_json(
            unknown_major.path(),
            "observed/exp-chrome-browser-client.json",
            |value| {
                value["payload_version"] = json!("host-capability-raw/2.0.0");
            },
        );
        assert!(evaluate_codex_phase1_fixture(unknown_major.path()).is_err());

        let missing_final_status = materialize_embedded_phase1_fixture().unwrap();
        mutate_json(
            missing_final_status.path(),
            "observed/exp-chrome-browser-client.json",
            |value| {
                value["events"][1]["payload"]
                    .as_object_mut()
                    .unwrap()
                    .remove("final_status");
            },
        );
        assert!(evaluate_codex_phase1_fixture(missing_final_status.path()).is_err());

        let artifact_drift = materialize_embedded_phase1_fixture().unwrap();
        fs::write(
            artifact_drift
                .path()
                .join("artifacts/chrome-browser-client/cdp-result.json"),
            b"{\"forged\":true}\n",
        )
        .unwrap();
        assert!(evaluate_codex_phase1_fixture(artifact_drift.path()).is_err());

        let manifest_drift = materialize_embedded_phase1_fixture().unwrap();
        fs::write(
            manifest_drift
                .path()
                .join("manifests/phase1-host-capability-manifests.json"),
            b"{\"forged\":true}\n",
        )
        .unwrap();
        assert!(evaluate_codex_phase1_fixture(manifest_drift.path()).is_err());
    }

    fn mutate_json(root: &Path, relative: &str, mutate: impl FnOnce(&mut Value)) {
        let path = root.join(relative);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        mutate(&mut value);
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}
