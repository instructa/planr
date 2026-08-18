use crate::evidence::model::{
    NamespacedIdentifier, PayloadSchemaBinding, Sha256Digest, VerificationCapabilityManifest,
};
use crate::canonical_json::sha256_json_digest;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const CODEX_CHROME_BROWSER_CLIENT_OBSERVATION: &str = "host.codex.chrome_browser_client";
const HOST_CAPTURE_SCHEMA_ID: &str = "planr.host_capability_observed_raw.schema.v1";
const HOST_CAPTURE_SCHEMA_REF: &str =
    "schemas/host-capability-observed-raw.schema.json";
const HOST_CAPTURE_SCHEMA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/host-capability-runtime/v1/schemas/host-capability-observed-raw.schema.json"
));

#[derive(Debug, Clone)]
struct BuiltInObservationSchema {
    binding: PayloadSchemaBinding,
    schema: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct BuiltInEvidenceCatalog {
    host_capture_schema: BuiltInObservationSchema,
    host_admission_manifest: VerificationCapabilityManifest,
}

impl BuiltInEvidenceCatalog {
    pub(crate) fn load() -> Result<Self> {
        let schema: Value = serde_json::from_str(HOST_CAPTURE_SCHEMA_JSON)
            .context("parsing built-in host capture schema")?;
        if schema.get("$id").and_then(Value::as_str) != Some(HOST_CAPTURE_SCHEMA_ID) {
            bail!("built-in host capture schema has an unexpected $id");
        }
        let schema_digest = Sha256Digest::parse(sha256_json_digest(&schema)?)
            .map_err(anyhow::Error::from)?;
        let binding = PayloadSchemaBinding {
            observation_type: NamespacedIdentifier::parse(
                CODEX_CHROME_BROWSER_CLIENT_OBSERVATION,
            )
            .map_err(anyhow::Error::from)?,
            schema_ref: HOST_CAPTURE_SCHEMA_REF.to_string(),
            schema_digest,
        };
        let host_admission_manifest = serde_json::from_value(json!({
            "id": "host-codex-chrome-browser-client-admission-v1",
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "host",
            "adapter_digest": binding.schema_digest.as_str(),
            "supported_surfaces": ["local-process"],
            "supported_observations": [binding.clone()],
            "supported_interactions": ["external-capture-admission"],
            "supported_artifacts": ["host-capture-envelope", "cdp-json-result"],
            "runtime_targets": [{
                "kind": "host-capture-admission",
                "id": "codex-chrome-browser-client-admission"
            }],
            "provenance_path": "verified_host_event",
            "permissions": {"network": "none", "filesystem": "read_workspace"},
            "costs": {"execution": "local-process"},
            "determinism": "validated-external-capture-dependent",
            "repeatability": "fresh-capture-required",
            "independence": "Planr admission validates an external host capture",
            "blind_spots": ["availability proves admission support, not a captured browser event"],
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
                    "payload_schema": binding.clone()
                }
            }
        }))
        .context("building built-in host admission capability manifest")?;
        Ok(Self {
            host_capture_schema: BuiltInObservationSchema { binding, schema },
            host_admission_manifest,
        })
    }

    pub(crate) fn manifests(&self) -> impl Iterator<Item = VerificationCapabilityManifest> + '_ {
        std::iter::once(self.host_admission_manifest.clone())
    }

    pub(crate) fn is_admission_bootstrap_manifest(
        &self,
        manifest: &VerificationCapabilityManifest,
    ) -> Result<bool> {
        if manifest.id != self.host_admission_manifest.id {
            return Ok(false);
        }
        if manifest.adapter_kind != self.host_admission_manifest.adapter_kind
            || manifest.supported_interactions
                != self.host_admission_manifest.supported_interactions
        {
            bail!("built-in host admission bootstrap manifest semantics changed");
        }
        Ok(true)
    }

    pub(crate) fn resolve_schema(
        &self,
        observation_type: &NamespacedIdentifier,
        schema_ref: &str,
        schema_digest: Option<&Sha256Digest>,
    ) -> Result<Option<&Value>> {
        let binding = &self.host_capture_schema.binding;
        let claims_builtin = observation_type == &binding.observation_type
            || schema_ref == binding.schema_ref;
        if !claims_builtin {
            return Ok(None);
        }
        if observation_type != &binding.observation_type
            || schema_ref != binding.schema_ref
            || schema_digest.is_some_and(|digest| digest != &binding.schema_digest)
        {
            bail!(
                "reserved built-in observation must use the exact {} schema binding",
                binding.observation_type.as_str()
            );
        }
        Ok(Some(&self.host_capture_schema.schema))
    }
}
