use super::App;
use super::repository::execution_run::{
    ExecutionRunRepository, FindingStatus, ReviewGateKind, ReviewGateStatus,
    ReviewSourceBindingRecord, SourceFreezeRecord,
};
use crate::cli::{
    EvidenceCapabilityCommand, EvidenceCommand, EvidenceCoverageScope, EvidenceHostCaptureCommand,
    EvidenceObligationCommand,
};
use crate::evidence::model::{
    ArtifactRef, CapabilityBinding, EvidenceAttempt, EvidenceReceipt, ObservationRequirement,
    ObservationResult, RawResultRef, SandboxLimits, SandboxState, Sha256Digest, TrustedProvenance,
    TrustedReceiptInput, VantagePoint, build_trusted_receipt,
};
use crate::evidence::{
    AttemptStatus, CapabilityRegistry, CapabilityRuntimeContext, EnvironmentBinding, EvidenceId,
    FixtureDisclosure, GapReason, ProcessExecutionContract, ProofObligation, ProvenanceSourceKind,
    TargetBinding, ValidatedArtifactImportRepository, VerificationCapabilityInstance,
    VerificationCapabilityManifest,
    coverage::{
        AuthoritativeObligationBindingRow, CoverageEvaluation,
        authoritative_obligation_bindings_for_scope, authoritative_obligation_ids_for_scope,
        canonical_coverage_projection, evaluate_criterion_coverage, evaluate_item_coverage,
        evaluate_item_criterion_coverages, evaluate_obligation_coverage, evaluate_plan_coverage,
        evaluate_plan_criterion_coverages,
    },
    execution::{
        ConfiguredProcessRunInput, TrustedEvidencePersistenceInput, ensure_process_adapter_digest,
        persist_trusted_evidence_atomically, resolve_process_run,
        run_configured_process_adapter_guarded, run_repository_snapshot_pre_commit_test_hook,
        run_resolved_process, select_execution_binding_subset,
    },
    parse_validated_artifact_import,
    policy::{
        capture_repository_snapshot, load_repository_observation_schema,
        parse_evidence_policy_yaml, parse_trusted_receipt_binding,
    },
};
use crate::execution::{BoundedProcessInput, CancellationToken, run_bounded_process};
use crate::execution_run::{
    ExecutionBatch, ExecutionBatchStatus, FeatureRunPhase, PhaseTransition, PhaseTransitionCause,
    RoleOwner, RunRole, apply_phase_transition,
};
use crate::util::short_id;
use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::fmt;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(crate) const EVIDENCE_OK: i32 = 0;
pub(crate) const EVIDENCE_UNSATISFIED: i32 = 2;
pub(crate) const EVIDENCE_BLOCKED: i32 = 3;
pub(crate) const EVIDENCE_ERROR: i32 = 1;
const HOST_CAPTURE_VALIDATOR_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug)]
pub(crate) enum CurrentPlanCoverageForSourceFreeze {
    NeedsVerification(CoverageEvaluation),
    Satisfied(CoverageEvaluation),
}

#[derive(Debug)]
struct CoverageSourceFreezeMismatch {
    freeze_id: String,
    receipt_id: String,
}

impl std::fmt::Display for CoverageSourceFreezeMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "verification_coverage_source_freeze_mismatch:{}:{}",
            self.freeze_id, self.receipt_id
        )
    }
}

impl std::error::Error for CoverageSourceFreezeMismatch {}

struct HermeticReuseBinding {
    key: String,
    execution_contract_digest: String,
    source_tree_digest: String,
    toolchain_lock_digest: String,
    execution_binding: Value,
}

struct HermeticReuseInput<'a> {
    obligation: &'a ProofObligation,
    instance: &'a VerificationCapabilityInstance,
    execution_contract: &'a ProcessExecutionContract,
    target: &'a TargetBinding,
    environment: &'a EnvironmentBinding,
    fixture_disclosure: &'a FixtureDisclosure,
    env: &'a BTreeMap<String, String>,
    execution_binding: &'a Value,
}

fn is_hermetic_reuse_candidate(
    manifest: &VerificationCapabilityManifest,
    target: &TargetBinding,
    fixture_disclosure: &FixtureDisclosure,
    env: &BTreeMap<String, String>,
) -> Result<bool> {
    if fixture_disclosure.fixtures_used
        || fixture_disclosure.mocks_used
        || fixture_disclosure.fixture_refs.is_some()
        || fixture_disclosure.mock_refs.is_some()
        || !env.is_empty()
    {
        return Ok(false);
    }
    let network_is_none =
        manifest.permissions.get("network").and_then(Value::as_str) == Some("none");
    let deterministic =
        manifest.determinism == "deterministic" && manifest.repeatability == "repeatable";
    let runtime_words = serde_json::to_string(&json!({
        "surfaces": manifest.supported_surfaces,
        "interactions": manifest.supported_interactions,
        "targets": manifest.runtime_targets,
        "target": target,
    }))?
    .to_ascii_lowercase();
    let forbidden_runtime = ["browser", "http", "network", "live", "websocket"]
        .iter()
        .any(|word| runtime_words.contains(word));
    Ok(network_is_none && deterministic && !forbidden_runtime)
}

#[derive(Debug)]
pub(crate) struct EvidenceCliExit {
    code: i32,
    message: String,
    emitted: bool,
}

impl EvidenceCliExit {
    fn new(code: i32, message: impl Into<String>, emitted: bool) -> Self {
        Self {
            code,
            message: message.into(),
            emitted,
        }
    }

    pub(crate) fn code(&self) -> i32 {
        self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn emitted(&self) -> bool {
        self.emitted
    }
}

impl std::fmt::Display for EvidenceCliExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EvidenceCliExit {}

#[derive(Debug)]
pub(crate) struct EvidenceCommandError {
    code: &'static str,
    message: String,
}

impl EvidenceCommandError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            code: "bad_request",
            message: message.into(),
        }
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: "conflict",
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal_error",
            message: message.into(),
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for EvidenceCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EvidenceCommandError {}

impl App {
    pub(crate) fn evidence(&self, command: EvidenceCommand) -> Result<()> {
        let (command_name, result, human) = match command {
            EvidenceCommand::Policy(args) => (
                "evidence.policy",
                self.evidence_policy_value(),
                if args.check {
                    "evidence policy checked".to_string()
                } else {
                    "evidence policy".to_string()
                },
            ),
            EvidenceCommand::Obligation(args) => match args.command {
                EvidenceObligationCommand::List(args) => (
                    "evidence.obligation.list",
                    self.evidence_obligations_value(
                        args.plan.as_deref(),
                        args.item.as_deref(),
                        args.criterion.as_deref(),
                    ),
                    "evidence obligations".to_string(),
                ),
                EvidenceObligationCommand::Show(args) => (
                    "evidence.obligation.show",
                    self.evidence_obligation_value(&args.id),
                    format!("evidence obligation {}", args.id),
                ),
            },
            EvidenceCommand::Capability(args) => match args.command {
                EvidenceCapabilityCommand::List => (
                    "evidence.capability.list",
                    self.evidence_capabilities_value(),
                    "evidence capabilities".to_string(),
                ),
                EvidenceCapabilityCommand::Show(args) => (
                    "evidence.capability.show",
                    self.evidence_capability_value(&args.id),
                    format!("evidence capability {}", args.id),
                ),
            },
            EvidenceCommand::Run(args) => {
                let value = read_json_file(&args.input)?;
                (
                    "evidence.run",
                    self.evidence_run_value(value),
                    "evidence run".to_string(),
                )
            }
            EvidenceCommand::Import(args) => {
                let value = read_json_file(&args.input)?;
                (
                    "evidence.import",
                    self.evidence_import_value(value, &args.artifact_root),
                    "evidence import".to_string(),
                )
            }
            EvidenceCommand::HostCapture(args) => match args.command {
                EvidenceHostCaptureCommand::Import(input) => {
                    let value = read_json_file(&input.input)?;
                    (
                        "evidence.host_capture.import",
                        self.evidence_host_capture_import_value(value),
                        "evidence host capture import".to_string(),
                    )
                }
                EvidenceHostCaptureCommand::Run(input) => {
                    let value = read_json_file(&input.input)?;
                    (
                        "evidence.host_capture.run",
                        self.evidence_host_capture_run_value(value),
                        "evidence host capture run".to_string(),
                    )
                }
            },
            EvidenceCommand::Attempts(args) => (
                "evidence.attempts",
                self.evidence_attempts_value(args.id.as_deref(), args.obligation.as_deref()),
                "evidence attempts".to_string(),
            ),
            EvidenceCommand::Receipts(args) => (
                "evidence.receipts",
                self.evidence_receipts_value(args.id.as_deref(), args.obligation.as_deref()),
                "evidence receipts".to_string(),
            ),
            EvidenceCommand::Coverage(args) => (
                "evidence.coverage",
                self.evidence_coverage_value(args.scope, &args.id),
                "evidence coverage".to_string(),
            ),
            EvidenceCommand::Explain(args) => (
                "evidence.explain",
                self.evidence_explain_value(args.scope, &args.id),
                "evidence explanation".to_string(),
            ),
            EvidenceCommand::Readiness(args) => (
                "evidence.readiness",
                self.evidence_readiness_value(args.scope, &args.id),
                "evidence readiness".to_string(),
            ),
            EvidenceCommand::RecoverSettlement(args) => {
                let value = read_json_file(&args.input)?;
                (
                    "evidence.recover_settlement",
                    self.recover_verification_settlement_value(value),
                    "verification settlement recovery".to_string(),
                )
            }
            EvidenceCommand::Migrate(args) => {
                let value = read_json_file(&args.input)?;
                (
                    "evidence.migrate",
                    self.evidence_migration_value(value, args.apply),
                    if args.apply {
                        "evidence migration applied".to_string()
                    } else {
                        "evidence migration preview".to_string()
                    },
                )
            }
            EvidenceCommand::Classifications => (
                "evidence.classifications",
                Ok(evidence_classifications_value()),
                "evidence classifications".to_string(),
            ),
        };
        self.emit_evidence_result(command_name, result, human)
    }

    pub(crate) fn emit_evidence_result(
        &self,
        command: &'static str,
        result: Result<Value>,
        human: String,
    ) -> Result<()> {
        match result {
            Ok(object) => {
                let envelope = evidence_success_envelope(command, object);
                let exit_code = evidence_envelope_exit_code(&envelope);
                self.emit(envelope, human)?;
                if exit_code == 0 {
                    Ok(())
                } else {
                    Err(EvidenceCliExit::new(
                        exit_code,
                        format!("evidence {command} did not pass"),
                        true,
                    )
                    .into())
                }
            }
            Err(error) => {
                let message = error.to_string();
                let envelope = evidence_error_envelope(command, &error);
                if self.json {
                    crate::util::print_json(&envelope)?;
                    Err(EvidenceCliExit::new(EVIDENCE_ERROR, message, true).into())
                } else {
                    Err(EvidenceCliExit::new(EVIDENCE_ERROR, message, false).into())
                }
            }
        }
    }

    pub(crate) fn evidence_policy_value(&self) -> Result<Value> {
        let Some(document) = self.evidence_policy_document()? else {
            return Ok(json!({
                "policy": null,
                "status": "absent",
                "diagnostics": [{"code": "missing_policy", "message": ".planr/evidence.yaml is not present"}],
            }));
        };
        let mut registry = self.evidence_registry_from_policy(&document)?;
        let probe = self.probe_registry_capabilities(&mut registry)?;
        Ok(json!({
            "policy": serde_json::to_value(&document.policy)?,
            "digest": document.digest,
            "waivers": document.waivers,
            "registry": probe,
            "status": if registry.diagnostics().is_empty() { "valid" } else { "warning" },
            "diagnostics": registry_diagnostics_value(registry.diagnostics()),
        }))
    }

    fn evidence_policy_document(
        &self,
    ) -> Result<Option<crate::evidence::policy::EvidencePolicyDocument>> {
        let path = self.root.join(".planr/evidence.yaml");
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        parse_evidence_policy_yaml(&text)
            .map(Some)
            .map_err(|diagnostics| anyhow!("evidence policy invalid: {diagnostics}"))
    }

    pub(crate) fn evidence_policy_requires_binding(&self) -> Result<bool> {
        Ok(self
            .evidence_policy_document()?
            .map(|document| {
                document.policy.defaults["binding"]
                    .as_bool()
                    .unwrap_or(true)
            })
            .unwrap_or(false))
    }

    fn evidence_policy_doctor_value(&self) -> Value {
        match self.evidence_policy_value() {
            Ok(mut policy) => {
                let state = evidence_doctor_policy_state(&policy);
                if let Some(object) = policy.as_object_mut() {
                    object.insert("state".to_string(), json!(state));
                }
                policy
            }
            Err(error) => {
                let message = error.to_string();
                let state = if policy_text_has_no_adapters(&self.root) {
                    "no_adapters"
                } else {
                    "malformed_policy"
                };
                json!({
                    "policy": null,
                    "status": "invalid",
                    "state": state,
                    "registry": empty_registry_probe(json!([{
                        "manifest_id": "policy",
                        "code": "policy_invalid",
                        "message": message,
                    }])),
                    "diagnostics": [{
                        "code": "policy_invalid",
                        "message": message,
                    }],
                })
            }
        }
    }

    fn evidence_registry_from_policy(
        &self,
        document: &crate::evidence::policy::EvidencePolicyDocument,
    ) -> Result<CapabilityRegistry> {
        Ok(
            CapabilityRegistry::from_manifests_and_adapter_registrations(
                &self.root,
                std::iter::empty(),
                &document.policy.adapter_registrations,
            ),
        )
    }

    fn default_capability_runtime(&self) -> CapabilityRuntimeContext<'static> {
        CapabilityRuntimeContext {
            host: "planr",
            surface: "local-process",
            host_version: env!("CARGO_PKG_VERSION"),
            environment_id: "planr-local",
        }
    }

    fn probe_registry_capabilities(&self, registry: &mut CapabilityRegistry) -> Result<Value> {
        let runtime = self.default_capability_runtime();
        let mut probes = Vec::new();
        let manifest_ids = registry
            .capabilities()
            .map(|capability| capability.manifest.id.as_str().to_string())
            .collect::<Vec<_>>();
        for manifest_id in manifest_ids {
            let outcome = registry
                .current_or_probe_and_store(&self.conn, &self.root, &manifest_id, runtime)
                .map(|resolution| {
                    json!({
                        "manifest_id": manifest_id,
                        "instance_id": resolution.instance.id.as_str(),
                        "availability_status": resolution.instance.availability.status.as_str(),
                        "reused": resolution.reused,
                        "resolution": resolution.reason.as_str(),
                    })
                })
                .unwrap_or_else(|error| {
                    json!({
                        "manifest_id": manifest_id,
                        "availability_status": "unavailable",
                        "error": error.to_string(),
                        "reused": false,
                        "resolution": "error",
                    })
                });
            probes.push(outcome);
        }
        Ok(json!({
            "registered_capabilities": registry.capabilities().count(),
            "available_capabilities": probes.iter().filter(|probe| probe["availability_status"] == "available").count(),
            "probes": probes,
            "diagnostics": registry_diagnostics_value(registry.diagnostics()),
        }))
    }

    fn insert_migrated_evidence_obligation(&self, obligation: &ProofObligation) -> Result<Value> {
        // Sole production writer for ProofObligation rows. Callers must pass
        // through the atomic, plan-scoped migration contract below.
        let project = self.default_project()?;
        let obligation_id = obligation.id.clone();
        let obligation_version = self.next_obligation_version(&project.id, obligation)?;
        let semantic_digest =
            crate::canonical_json::sha256_json_digest(&serde_json::to_value(obligation)?)?;
        self.conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, retry_aggregation, policy_digest, config_digest,
              source_digest, supersedes_obligation_id, created_at, obligation_shape
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, NULL, ?15, datetime('now'), 'semantic_v1')",
            params![
                obligation.id.as_str(),
                project.id,
                obligation.plan_id.as_str(),
                obligation.item_id.as_ref().map(|id| id.as_str()),
                obligation.criterion_id.as_str(),
                obligation_version,
                obligation.title.as_str(),
                if obligation.binding { 1 } else { 0 },
                serde_json::to_string(&obligation.observations)?,
                obligation.fixture_policy.to_string(),
                obligation.freshness_policy.to_string(),
                obligation.assurance_policy.to_string(),
                obligation_retry_aggregation(obligation)?,
                semantic_digest,
                obligation.supersedes.as_ref().map(|id| id.as_str()),
            ],
        )?;
        Ok(json!({
            "obligation": self.evidence_obligation_record_value(obligation_id.as_str())?,
            "verdict": "valid",
        }))
    }

    pub(crate) fn evidence_migration_value(&self, value: Value, apply: bool) -> Result<Value> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT evidence_migration")?;
        let result = self.evidence_migration_value_inner(value, apply);
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE evidence_migration; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO evidence_migration; RELEASE evidence_migration; COMMIT",
                );
                Err(error)
            }
        }
    }

    fn evidence_migration_value_inner(&self, value: Value, apply: bool) -> Result<Value> {
        let object = value.as_object().ok_or_else(|| {
            EvidenceCommandError::bad_request("evidence migration input must be a JSON object")
        })?;
        let allowed = BTreeSet::from(["schema_version", "plan_id", "obligations"]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(EvidenceCommandError::bad_request(format!(
                "evidence migration input has unknown fields: {}",
                unknown.join(",")
            ))
            .into());
        }
        for required in ["schema_version", "plan_id", "obligations"] {
            if !object.contains_key(required) {
                return Err(EvidenceCommandError::bad_request(format!(
                    "evidence migration input requires {required}"
                ))
                .into());
            }
        }
        let schema_version = object
            .get("schema_version")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                EvidenceCommandError::bad_request(
                    "evidence migration schema_version must be a string",
                )
            })?;
        if schema_version != "planr.evidence.migration.v1" {
            return Err(EvidenceCommandError::bad_request(format!(
                "unsupported evidence migration schema_version: {schema_version}"
            ))
            .into());
        }
        let plan_id = string_field(&value, "plan_id")?;
        let plan = self.get_plan(&plan_id)?;
        let project = self.default_project()?;
        if plan.project_id != project.id {
            return Err(EvidenceCommandError::bad_request(
                "evidence migration plan does not belong to current project",
            )
            .into());
        }
        let obligations = object
            .get("obligations")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EvidenceCommandError::bad_request("evidence migration input requires obligations[]")
            })?;
        if obligations.is_empty() {
            return Err(EvidenceCommandError::bad_request(
                "evidence migration input requires at least one obligation",
            )
            .into());
        }

        let mut parsed = Vec::new();
        let mut preview = Vec::new();
        let mut warnings = Vec::new();
        let verification_claims = self.verification_logs_for_plan(&plan_id)?;
        if !verification_claims.is_empty() {
            warnings.push(json!({
                "code": "verification_claims_are_not_evidence",
                "message": "verification logs remain visible claim-only diagnostics and do not satisfy binding Evidence",
                "count": verification_claims.len(),
            }));
        }
        let mut seen_ids = BTreeSet::new();
        let mut seen_lineage = BTreeMap::new();
        for raw in obligations {
            let obligation: ProofObligation =
                serde_json::from_value(raw.clone()).map_err(|error| {
                    EvidenceCommandError::bad_request(format!(
                        "evidence migration obligation is invalid: {error}"
                    ))
                })?;
            if !seen_ids.insert(obligation.id.as_str().to_string()) {
                return Err(EvidenceCommandError::bad_request(format!(
                    "evidence migration input contains duplicate obligation id: {}",
                    obligation.id.as_str()
                ))
                .into());
            }
            self.validate_migration_obligation(&plan, &obligation)
                .map_err(|error| EvidenceCommandError::bad_request(error.to_string()))?;
            let obligation_version = self.next_obligation_version(&project.id, &obligation)?;
            let lineage_key = (
                obligation.plan_id.as_str().to_string(),
                obligation.criterion_id.as_str().to_string(),
                obligation_version,
            );
            let existing = self
                .conn
                .query_row(
                    "SELECT id, project_id, plan_id, item_id, criterion_id, title, binding,
                            observation_requirements_json, fixture_policy_json, freshness_policy_json,
                            assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
                            supersedes_obligation_id, obligation_version, created_at, obligation_shape
                     FROM proof_obligations WHERE id = ?1",
                    params![obligation.id.as_str()],
                    obligation_row,
                )
                .optional()?;
            let action = match existing {
                None => "create",
                Some(existing)
                    if existing["project_id"].as_str() == Some(project.id.as_str())
                        && existing_obligation_matches(&existing, &obligation) =>
                {
                    "unchanged"
                }
                Some(existing) => {
                    preview.push(json!({
                        "id": obligation.id.as_str(),
                        "criterion_id": obligation.criterion_id.as_str(),
                        "action": "conflict",
                        "reason": if existing["project_id"].as_str() == Some(project.id.as_str()) {
                            "changed_payload"
                        } else {
                            "global_obligation_id_collision"
                        },
                        "existing_project_id": existing["project_id"],
                        "existing_plan_id": existing["plan_id"],
                        "existing_criterion_id": existing["criterion_id"],
                    }));
                    parsed.push(obligation);
                    continue;
                }
            };
            if action == "create" {
                if let Some(existing_id) = seen_lineage.get(&lineage_key) {
                    preview.push(json!({
                        "id": obligation.id.as_str(),
                        "criterion_id": obligation.criterion_id.as_str(),
                        "action": "conflict",
                        "reason": "batch_lineage_identity_collision",
                        "existing_id": existing_id,
                        "obligation_version": obligation_version,
                    }));
                    parsed.push(obligation);
                    continue;
                }
                if let Some(existing_lineage) = self.existing_obligation_for_lineage(
                    &project.id,
                    obligation.plan_id.as_str(),
                    obligation.criterion_id.as_str(),
                    obligation_version,
                )? {
                    preview.push(json!({
                        "id": obligation.id.as_str(),
                        "criterion_id": obligation.criterion_id.as_str(),
                        "action": "conflict",
                        "reason": "lineage_identity_collision",
                        "existing_id": existing_lineage["id"],
                        "obligation_version": obligation_version,
                    }));
                    parsed.push(obligation);
                    continue;
                }
                seen_lineage.insert(lineage_key, obligation.id.as_str().to_string());
            } else if action == "unchanged" {
                seen_lineage.insert(lineage_key, obligation.id.as_str().to_string());
            }
            preview.push(json!({
                "id": obligation.id.as_str(),
                "criterion_id": obligation.criterion_id.as_str(),
                "item_id": obligation.item_id.as_ref().map(|id| id.as_str()),
                "binding": obligation.binding,
                "observations": obligation.observations.len(),
                "action": action,
                "obligation_version": obligation_version,
            }));
            parsed.push(obligation);
        }
        let candidate_bindings = parsed
            .iter()
            .map(|obligation| {
                (
                    obligation.id.as_str().to_string(),
                    obligation.criterion_id.as_str().to_string(),
                )
            })
            .collect::<Vec<_>>();
        self.require_complete_plan_criterion_bindings(&plan_id, &candidate_bindings)
            .map_err(|error| EvidenceCommandError::bad_request(error.to_string()))?;
        let conflicts = preview
            .iter()
            .filter(|entry| entry["action"].as_str() == Some("conflict"))
            .count();
        if conflicts > 0 {
            for entry in &mut preview {
                if entry["action"].as_str() == Some("create") {
                    entry["action"] = json!("blocked");
                    entry["reason"] = json!("batch_has_conflicts");
                }
            }
        }
        if conflicts > 0 && apply {
            return Err(EvidenceCommandError::conflict(format!(
                "evidence migration has {conflicts} conflict(s); preview and resolve before apply"
            ))
            .into());
        }

        let mut created = Vec::new();
        if apply {
            let fail_after_creates = std::env::var("PLANR_EVIDENCE_MIGRATION_FAIL_AFTER_CREATES")
                .ok()
                .and_then(|value| value.parse::<usize>().ok());
            let result = (|| -> Result<()> {
                for obligation in parsed {
                    if preview.iter().any(|entry| {
                        entry["id"].as_str() == Some(obligation.id.as_str())
                            && entry["action"].as_str() == Some("create")
                    }) {
                        created.push(
                            self.insert_migrated_evidence_obligation(&obligation)?["obligation"]
                                .clone(),
                        );
                        if fail_after_creates == Some(created.len()) {
                            bail!(
                                "injected evidence migration failure after {} create(s)",
                                created.len()
                            );
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                return Err(EvidenceCommandError::internal(error.to_string()).into());
            }
        }

        Ok(json!({
            "schema_version": "planr.evidence.migration.v1",
            "plan_id": plan_id,
            "dry_run": !apply,
            "status": if apply { "applied" } else { "preview" },
            "verdict": "valid",
            "summary": {
                "create": preview.iter().filter(|entry| entry["action"] == "create").count(),
                "unchanged": preview.iter().filter(|entry| entry["action"] == "unchanged").count(),
                "conflict": conflicts,
                "blocked": preview.iter().filter(|entry| entry["action"] == "blocked").count(),
            },
            "obligations": preview,
            "created": created,
            "verification_claims": verification_claims,
            "warnings": warnings,
            "classifications": evidence_classifications_value(),
            "next_action": if apply {
                format!("run planr evidence coverage --scope plan --id {plan_id}")
            } else {
                format!("rerun planr evidence migrate --input <migration-file> --apply to bind plan {plan_id} Evidence obligations")
            },
        }))
    }

    fn validate_migration_obligation(
        &self,
        plan: &crate::model::Plan,
        obligation: &ProofObligation,
    ) -> Result<()> {
        if obligation.plan_id.as_str() != plan.id {
            bail!(
                "migration obligation {} targets a different plan",
                obligation.id.as_str()
            );
        }
        if !obligation.binding {
            bail!(
                "migration obligation {} must be binding=true; non-binding pre-Evidence behavior is preserved without migration",
                obligation.id.as_str()
            );
        }
        if let Some(item_id) = &obligation.item_id {
            let item = self.get_item(item_id.as_str())?;
            if item.project_id != plan.project_id
                || item.plan_path.as_deref() != Some(plan.path.as_str())
            {
                bail!(
                    "migration obligation {} item {} does not belong to project {} plan {}",
                    obligation.id.as_str(),
                    item_id.as_str(),
                    plan.project_id,
                    plan.id
                );
            }
        }
        Ok(())
    }

    fn existing_obligation_for_lineage(
        &self,
        project_id: &str,
        plan_id: &str,
        criterion_id: &str,
        obligation_version: i64,
    ) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT id, project_id, plan_id, item_id, criterion_id, title, binding,
                        observation_requirements_json, fixture_policy_json, freshness_policy_json,
                        assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
                        supersedes_obligation_id, obligation_version, created_at, obligation_shape
                 FROM proof_obligations
                 WHERE project_id = ?1 AND plan_id = ?2 AND criterion_id = ?3 AND obligation_version = ?4",
                params![project_id, plan_id, criterion_id, obligation_version],
                obligation_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn verification_logs_for_plan(&self, plan_id: &str) -> Result<Vec<Value>> {
        let plan = self.get_plan(plan_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT logs.id, logs.item_id, logs.summary, logs.created_at
             FROM logs
             JOIN items ON items.id = logs.item_id
             WHERE items.project_id = ?1
               AND items.plan_path = ?2
               AND logs.kind = 'verification'
             ORDER BY logs.created_at, logs.id",
        )?;
        let rows = stmt.query_map(params![plan.project_id, plan.path], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "item_id": row.get::<_, String>(1)?,
                "summary": row.get::<_, String>(2)?,
                "created_at": row.get::<_, String>(3)?,
                "authority": "claim_only",
            }))
        })?;
        crate::util::collect_rows(rows)
    }

    fn next_obligation_version(
        &self,
        project_id: &str,
        obligation: &ProofObligation,
    ) -> Result<i64> {
        let Some(supersedes) = &obligation.supersedes else {
            return Ok(1);
        };
        self.conn
            .query_row(
                "SELECT obligation_version + 1
                 FROM proof_obligations
                 WHERE project_id = ?1
                   AND id = ?2
                   AND plan_id = ?3
                   AND criterion_id = ?4",
                params![
                    project_id,
                    supersedes.as_str(),
                    obligation.plan_id.as_str(),
                    obligation.criterion_id.as_str()
                ],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("superseded evidence obligation not found or mismatched"))
    }

    pub(crate) fn evidence_obligations_value(
        &self,
        plan: Option<&str>,
        item: Option<&str>,
        criterion: Option<&str>,
    ) -> Result<Value> {
        let project = self.default_project()?;
        let mut conditions = vec!["project_id = ?".to_string()];
        let mut args: Vec<String> = vec![project.id];
        if let Some(plan) = plan {
            conditions.push("plan_id = ?".to_string());
            args.push(plan.to_string());
        }
        if let Some(item) = item {
            conditions.push("item_id = ?".to_string());
            args.push(item.to_string());
        }
        if let Some(criterion) = criterion {
            conditions.push("criterion_id = ?".to_string());
            args.push(criterion.to_string());
        }
        let where_clause = format!(" WHERE {}", conditions.join(" AND "));
        let sql = format!(
            "SELECT id, project_id, plan_id, item_id, criterion_id, title, binding,
                    observation_requirements_json, fixture_policy_json, freshness_policy_json,
                    assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
                    supersedes_obligation_id, obligation_version, created_at, obligation_shape
             FROM proof_obligations{where_clause} ORDER BY created_at, id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), obligation_row)?;
        Ok(json!({"obligations": crate::util::collect_rows(rows)?}))
    }

    pub(crate) fn evidence_obligation_value(&self, id: &str) -> Result<Value> {
        Ok(json!({"obligation": self.evidence_obligation_record_value(id)?}))
    }

    pub(crate) fn evidence_capabilities_value(&self) -> Result<Value> {
        let policy = self.evidence_policy_document()?;
        let registry_probe = if let Some(document) = policy.as_ref() {
            let mut registry = self.evidence_registry_from_policy(document)?;
            self.probe_registry_capabilities(&mut registry)?
        } else {
            json!({
                "registered_capabilities": 0,
                "probes": [],
                "diagnostics": [{"manifest_id": "policy", "code": "missing_policy", "message": ".planr/evidence.yaml is not present"}],
            })
        };
        let manifests = query_json_rows(
            &self.conn,
            "SELECT id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, source_path, created_at
             FROM verification_capability_manifests ORDER BY id, version",
            [],
            manifest_row,
        )?;
        let instances = query_json_rows(
            &self.conn,
            "SELECT id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
                    availability_status, runtime_target_json, host_fingerprint_json,
                    capability_snapshot_json, probe_result_json, created_at, valid_until
             FROM verification_capability_instances ORDER BY created_at, id",
            [],
            capability_instance_row,
        )?;
        Ok(json!({"manifests": manifests, "instances": instances, "registry": registry_probe}))
    }

    pub(crate) fn evidence_capability_value(&self, id: &str) -> Result<Value> {
        let value = self
            .conn
            .query_row(
                "SELECT id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
                        availability_status, runtime_target_json, host_fingerprint_json,
                        capability_snapshot_json, probe_result_json, created_at, valid_until
                 FROM verification_capability_instances WHERE id = ?1",
                params![id],
                capability_instance_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("evidence capability instance not found: {id}"))?;
        Ok(json!({"capability": value}))
    }

    pub(crate) fn evidence_readiness_value(
        &self,
        scope: EvidenceCoverageScope,
        id: &str,
    ) -> Result<Value> {
        if matches!(scope, EvidenceCoverageScope::Plan) {
            match self.plan_evidence_authority(id)? {
                super::proof::PlanEvidenceAuthority::BindingActive => {}
                super::proof::PlanEvidenceAuthority::BindingUnsatisfied => {
                    let proof = self.proof_status_for_plan(id)?;
                    return Ok(json!({
                        "status": "blocked",
                        "scope": {"kind": "plan", "id": id},
                        "active_obligation_ids": [],
                        "observation_types": [],
                        "registry": null,
                        "feature_run_freeze": null,
                        "feature_run_readiness": null,
                        "run_index": null,
                        "gaps": proof["actionable_gaps"],
                        "proof": proof,
                        "next_action": proof["next_action"],
                    }));
                }
                super::proof::PlanEvidenceAuthority::NonBinding => {
                    return Err(EvidenceCommandError::bad_request(
                        "nonbinding plans do not enter Evidence readiness; freeze source and open final review",
                    )
                    .into());
                }
            }
        }
        let feature_run_freeze = if matches!(scope, EvidenceCoverageScope::Plan) {
            self.freeze_feature_run_source_value(id)?
        } else {
            None
        };
        if feature_run_freeze
            .as_ref()
            .is_some_and(|value| value["work_packet"]["kind"] == "hold")
        {
            return Ok(feature_run_freeze.expect("budget hold checked"));
        }
        // A plan-scoped readiness call may establish the canonical source freeze, but once the
        // FeatureRun is frozen nothing beyond that boundary may probe capabilities, parse policy,
        // or create an executable run index until the current worker owns the verifier lease.
        // Non-FeatureRun plans retain the ordinary readiness path.
        if matches!(scope, EvidenceCoverageScope::Plan) {
            self.resolve_feature_run_evidence_lease(&self.default_project()?.id, id)?;
        }
        let document = self.evidence_policy_document()?.ok_or_else(|| {
            EvidenceCommandError::bad_request("evidence readiness requires .planr/evidence.yaml")
        })?;
        let mut registry = self.evidence_registry_from_policy(&document)?;
        let probe = self.probe_registry_capabilities(&mut registry)?;
        let project = self.default_project()?;
        let active = authoritative_obligation_bindings_for_scope(
            &self.conn,
            &project.id,
            scope.as_str(),
            id,
        )
        .map_err(|error| anyhow!("{error}"))?;
        // The registry projection retains every repository diagnostic, but
        // readiness blocks only on capabilities needed by this active scope.
        let mut gaps = Vec::new();
        if active.is_empty() {
            gaps.push(json!({
                "code": "missing_obligation",
                "scope": scope.as_str(),
                "id": id,
                "message": "scope has no active binding proof obligations"
            }));
        }
        let mut observation_types = BTreeSet::new();
        for row in &active {
            let obligation_id = row.id.as_str();
            for observation in &row.observations {
                observation_types.insert(observation.observation_type.as_str().to_string());
                let Some(payload_schema) = observation.payload_schema.as_ref() else {
                    gaps.push(json!({
                        "code": "missing_payload_schema",
                        "obligation_id": obligation_id,
                        "requirement_id": observation.id.as_str(),
                        "message": "binding observation has no payload schema"
                    }));
                    continue;
                };
                match load_repository_observation_schema(
                    &self.root,
                    payload_schema.schema_ref.as_str(),
                ) {
                    Ok(Some(_)) => {}
                    Ok(None) => gaps.push(json!({
                        "code": "missing_payload_schema_registration",
                        "obligation_id": obligation_id,
                        "requirement_id": observation.id.as_str(),
                        "schema_ref": payload_schema.schema_ref,
                        "message": "repository payload schema is not registered"
                    })),
                    Err(error) => gaps.push(json!({
                        "code": "invalid_payload_schema",
                        "obligation_id": obligation_id,
                        "requirement_id": observation.id.as_str(),
                        "schema_ref": payload_schema.schema_ref,
                        "message": error.to_string()
                    })),
                }
                let matching = registry
                    .capabilities()
                    .filter(|capability| {
                        capability
                            .manifest
                            .supported_observations
                            .iter()
                            .any(|binding| {
                                binding.observation_type == observation.observation_type
                                    && binding.schema_ref == payload_schema.schema_ref
                            })
                    })
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    gaps.push(json!({
                        "code": "verifier_contract_mismatch",
                        "obligation_id": obligation_id,
                        "requirement_id": observation.id.as_str(),
                        "observation_type": observation.observation_type.as_str(),
                        "schema_ref": payload_schema.schema_ref,
                        "message": "no registered capability supports the observation type and payload schema"
                    }));
                    continue;
                }

                let mut available = false;
                let mut usable = false;
                let mut availability_statuses = BTreeSet::new();
                let mut adapter_digest_errors = Vec::new();
                for capability in matching {
                    let statuses = registry.availability_statuses_for(capability);
                    availability_statuses.extend(statuses.iter().copied());
                    if !statuses.contains("available") {
                        continue;
                    }
                    available = true;
                    let Some(execution) = capability.repository_execution_contract.as_ref() else {
                        usable = true;
                        continue;
                    };
                    match resolve_process_run(&self.root, execution, &BTreeMap::new()).and_then(
                        |resolved| ensure_process_adapter_digest(&capability.manifest, &resolved),
                    ) {
                        Ok(()) => usable = true,
                        Err(error) => adapter_digest_errors.push((
                            capability.manifest.id.as_str().to_string(),
                            error.to_string(),
                        )),
                    }
                }
                if !available {
                    gaps.push(json!({
                        "code": if availability_statuses.contains("permission_denied") {
                            "PermissionDenied"
                        } else {
                            "ProbeUnavailable"
                        },
                        "obligation_id": obligation_id,
                        "requirement_id": observation.id.as_str(),
                        "availability_statuses": availability_statuses,
                        "message": "no matching capability has a currently available runtime instance"
                    }));
                } else if !usable {
                    gaps.extend(
                        adapter_digest_errors
                            .into_iter()
                            .map(|(manifest_id, message)| {
                                json!({
                                    "code": "adapter_digest_drift",
                                    "manifest_id": manifest_id,
                                    "obligation_id": obligation_id,
                                    "requirement_id": observation.id.as_str(),
                                    "message": message,
                                })
                            }),
                    );
                }
            }
        }
        gaps.sort_by_key(|gap| gap.to_string());
        gaps.dedup();
        let feature_run_readiness = if matches!(scope, EvidenceCoverageScope::Plan) {
            self.classify_feature_run_readiness_value(id, !gaps.is_empty())?
        } else {
            None
        };
        let run_index = if gaps.is_empty() {
            Some(self.build_readiness_run_index(
                scope,
                id,
                &active,
                &registry,
                &probe,
                &document.digest,
            )?)
        } else {
            None
        };
        Ok(json!({
            "status": if gaps.is_empty() { "passed" } else { "blocked" },
            "scope": {"kind": scope.as_str(), "id": id},
            "active_obligation_ids": active.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            "observation_types": observation_types,
            "registry": probe,
            "feature_run_freeze": feature_run_freeze,
            "feature_run_readiness": feature_run_readiness,
            "run_index": run_index,
            "gaps": gaps,
            "next_action": if gaps.is_empty() {
                "run planr evidence run --input <exact-readiness.run_index.repository_path>"
            } else {
                "repair the reported Evidence policy, schema, capability, or runtime gap"
            }
        }))
    }

    fn build_readiness_run_index(
        &self,
        scope: EvidenceCoverageScope,
        scope_id: &str,
        active: &[AuthoritativeObligationBindingRow],
        registry: &CapabilityRegistry,
        probe: &Value,
        policy_digest: &str,
    ) -> Result<Value> {
        let available_instances = probe["probes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry["availability_status"] == "available")
            .filter_map(|entry| {
                Some((
                    entry["manifest_id"].as_str()?.to_string(),
                    entry["instance_id"].as_str()?.to_string(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let mut runs = Vec::new();
        for row in active {
            let partitions = canonical_target_partitions(&row.observations)?;
            let multi_target = partitions.len() > 1;
            for (target, requirement_ids) in partitions {
                let selected = row
                    .observations
                    .iter()
                    .filter(|observation| {
                        requirement_ids
                            .binary_search_by(|id| id.as_str().cmp(observation.id.as_str()))
                            .is_ok()
                    })
                    .collect::<Vec<_>>();
                let capability = registry
                    .capabilities()
                    .find(|capability| {
                        available_instances.contains_key(capability.manifest.id.as_str())
                            && selected.iter().all(|observation| {
                                observation.payload_schema.as_ref().is_some_and(|schema| {
                                    capability.manifest.supported_observations.iter().any(
                                        |binding| {
                                            binding.observation_type == observation.observation_type
                                                && binding.schema_ref == schema.schema_ref
                                        },
                                    )
                                })
                            })
                    })
                    .ok_or_else(|| {
                        anyhow!(
                            "no single available capability can execute target subset for {}",
                            row.id.as_str()
                        )
                    })?;
                if multi_target && capability.manifest.repeatability == "non_repeatable_one_shot" {
                    return Err(EvidenceCommandError::conflict(format!(
                        "multi-target obligation {} cannot use non_repeatable_one_shot capability {}",
                        row.id.as_str(), capability.manifest.id.as_str()
                    ))
                    .into());
                }
                let instance_id = available_instances
                    .get(capability.manifest.id.as_str())
                    .expect("available capability has probed instance");
                let instance = self.load_capability_instance(instance_id)?;
                let execution_contract = capability
                    .repository_execution_contract
                    .as_ref()
                    .unwrap_or(&capability.manifest.availability_probe.execution);
                runs.push(json!({
                    "index": runs.len(),
                    "capability": {
                        "instance_id": instance.id.as_str(),
                        "manifest_id": instance.manifest_id.as_str(),
                        "manifest_digest": instance.manifest_digest.as_str(),
                        "manifest_version": instance.adapter_version,
                    },
                    "input": {
                        "obligation_id": row.id.as_str(),
                        "requirement_ids": requirement_ids,
                        "capability_instance_id": instance_id,
                        "target": target,
                        "environment": instance.environment,
                        "execution_contract": execution_contract,
                        "fixture_disclosure": {
                            "fixtures_used": false,
                            "mocks_used": false
                        }
                    }
                }));
            }
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("capturing readiness run-index source: {error}"))?;
        let mut run_index = json!({
            "schema_version": "planr.evidence.run-index.v2",
            "scope": {"kind": scope.as_str(), "id": scope_id},
            "source": snapshot.source,
            "policy_digest": policy_digest,
            "runs": runs,
        });
        let digest = crate::canonical_json::sha256_json_digest(&run_index)?;
        let relative_path = format!(
            ".planr/evidence/runs/{}.json",
            digest.strip_prefix("sha256:").unwrap_or(&digest)
        );
        run_index["repository_path"] = json!(relative_path);
        let sealed_digest = crate::canonical_json::sha256_json_digest(&run_index)?;
        run_index["run_index_digest"] = json!(sealed_digest);
        let path = self.root.join(&relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_vec_pretty(&run_index)?)?;
        Ok(run_index)
    }

    fn validate_sealed_run_index(&self, value: &Value) -> Result<(String, Vec<Value>)> {
        let object = value.as_object().ok_or_else(|| {
            EvidenceCommandError::bad_request("evidence run-index input must be an object")
        })?;
        let allowed = BTreeSet::from([
            "schema_version",
            "scope",
            "source",
            "policy_digest",
            "runs",
            "repository_path",
            "run_index_digest",
        ]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(EvidenceCommandError::bad_request(format!(
                "evidence run-index input has unknown fields: {}",
                unknown.join(",")
            ))
            .into());
        }
        if value.get("schema_version").and_then(Value::as_str)
            != Some("planr.evidence.run-index.v2")
        {
            return Err(EvidenceCommandError::bad_request(
                "evidence run-index schema_version must be planr.evidence.run-index.v2",
            )
            .into());
        }
        let declared_digest = string_field(value, "run_index_digest")?;
        let actual_digest = crate::canonical_json::sha256_json_digest_without_top_level_field(
            value,
            "run_index_digest",
        )?;
        if declared_digest != actual_digest {
            return Err(
                EvidenceCommandError::conflict("evidence run-index seal is invalid").into(),
            );
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("checking evidence run-index source: {error}"))?;
        if value["source"] != serde_json::to_value(&snapshot.source)? {
            return Err(
                EvidenceCommandError::conflict("evidence run-index source is stale").into(),
            );
        }
        let document = self.evidence_policy_document()?.ok_or_else(|| {
            EvidenceCommandError::conflict("evidence run-index policy is unavailable")
        })?;
        if value["policy_digest"].as_str() != Some(document.digest.as_str()) {
            return Err(
                EvidenceCommandError::conflict("evidence run-index policy is stale").into(),
            );
        }
        let execution_bindings = self.validate_run_index_target_subsets(value, &declared_digest)?;
        Ok((declared_digest, execution_bindings))
    }

    fn evidence_run_index_value(&self, value: Value) -> Result<Value> {
        let (declared_digest, execution_bindings) = self.validate_sealed_run_index(&value)?;
        let runs = value["runs"]
            .as_array()
            .filter(|runs| !runs.is_empty())
            .ok_or_else(|| EvidenceCommandError::bad_request("evidence run-index has no runs"))?;
        let mut results = Vec::with_capacity(runs.len());
        let mut product_findings = Vec::new();
        for ((expected_index, run), execution_binding) in
            runs.iter().enumerate().zip(execution_bindings)
        {
            if run["index"].as_u64() != Some(expected_index as u64) {
                return Err(EvidenceCommandError::bad_request(
                    "evidence run-index entries must use contiguous indexes",
                )
                .into());
            }
            let input = run.get("input").cloned().ok_or_else(|| {
                EvidenceCommandError::bad_request("evidence run-index entry requires input")
            })?;
            let obligation_id = string_field(&input, "obligation_id")?;
            let result = self.evidence_run_single_value_with_routing(
                input,
                false,
                product_findings.is_empty(),
                execution_binding,
            )?;
            let product_failed = result["receipt"]["proof_gaps"]
                .as_array()
                .is_some_and(|gaps| {
                    gaps.iter().any(|gap| {
                        gap.as_str() == Some("product_failed")
                            || gap.get("reason").and_then(Value::as_str) == Some("product_failed")
                    })
                });
            let terminally_exhausted = !result["terminal_exhaustion"].is_null();
            if product_failed && !terminally_exhausted {
                let lease = result["feature_run_lease"].as_object().ok_or_else(|| {
                    anyhow!("sealed product finding is missing its FeatureRun lease")
                })?;
                product_findings.push((
                    expected_index,
                    string_field(&Value::Object(lease.clone()), "run_id")?,
                    string_field(&Value::Object(lease.clone()), "freeze_id")?,
                    obligation_id,
                ));
            }
            results.push(result);
            if terminally_exhausted {
                break;
            }
        }
        if let Some((_, run_id, freeze_id, _)) = product_findings.first() {
            if product_findings
                .iter()
                .any(|(_, candidate_run_id, candidate_freeze_id, _)| {
                    candidate_run_id != run_id || candidate_freeze_id != freeze_id
                })
            {
                bail!("sealed product findings span multiple FeatureRun leases");
            }
            let obligation_ids = product_findings
                .iter()
                .map(|(_, _, _, obligation_id)| obligation_id.clone())
                .collect::<Vec<_>>();
            let finding =
                self.route_evidence_product_findings_value(run_id, freeze_id, &obligation_ids)?;
            for (index, _, _, _) in &product_findings {
                results[*index]["product_finding"] = finding.clone();
            }
        }
        let verdict = evidence_run_index_verdict(&results);
        let terminal_exhaustion = results.iter().find_map(|result| {
            (!result["terminal_exhaustion"].is_null())
                .then(|| result["terminal_exhaustion"].clone())
        });
        Ok(json!({
            "schema_version": "planr.evidence.run-index.result.v2",
            "run_index_digest": declared_digest,
            "status": verdict,
            "verdict": verdict,
            "results": results,
            "terminal_exhaustion": terminal_exhaustion,
        }))
    }

    fn validate_run_index_target_subsets(
        &self,
        value: &Value,
        run_index_digest: &str,
    ) -> Result<Vec<Value>> {
        let scope_kind = value["scope"]["kind"]
            .as_str()
            .ok_or_else(|| EvidenceCommandError::bad_request("run-index scope.kind is required"))?;
        let scope_id = value["scope"]["id"]
            .as_str()
            .ok_or_else(|| EvidenceCommandError::bad_request("run-index scope.id is required"))?;
        let project = self.default_project()?;
        let active = authoritative_obligation_bindings_for_scope(
            &self.conn,
            &project.id,
            scope_kind,
            scope_id,
        )
        .map_err(|error| anyhow!("{error}"))?;
        let mut expected = BTreeMap::new();
        for row in active {
            for (target, requirement_ids) in canonical_target_partitions(&row.observations)? {
                let target_digest = crate::canonical_json::sha256_json_digest(&target)?;
                expected.insert((row.id.clone(), target_digest), (target, requirement_ids));
            }
        }
        let runs = value["runs"]
            .as_array()
            .filter(|runs| !runs.is_empty())
            .ok_or_else(|| EvidenceCommandError::bad_request("evidence run-index has no runs"))?;
        let mut seen = BTreeSet::new();
        let mut bindings = Vec::with_capacity(runs.len());
        for (index, run) in runs.iter().enumerate() {
            if run["index"].as_u64() != Some(index as u64) {
                return Err(EvidenceCommandError::bad_request(
                    "evidence run-index entries must use contiguous indexes",
                )
                .into());
            }
            let input = run["input"].as_object().ok_or_else(|| {
                EvidenceCommandError::bad_request("evidence run-index entry requires input")
            })?;
            let obligation_id = input
                .get("obligation_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    EvidenceCommandError::bad_request("run input obligation_id is required")
                })?;
            let target = input
                .get("target")
                .cloned()
                .ok_or_else(|| EvidenceCommandError::bad_request("run input target is required"))?;
            let target_digest = crate::canonical_json::sha256_json_digest(&target)?;
            let key = (obligation_id.to_string(), target_digest);
            if !seen.insert(key.clone()) {
                return Err(EvidenceCommandError::bad_request(
                    "run-index contains a duplicate obligation target subset",
                )
                .into());
            }
            let (expected_target, expected_requirement_ids) =
                expected.get(&key).ok_or_else(|| {
                    EvidenceCommandError::bad_request(
                        "run-index contains a foreign obligation target subset",
                    )
                })?;
            if &target != expected_target {
                return Err(EvidenceCommandError::bad_request(
                    "run-index target does not match the canonical obligation partition",
                )
                .into());
            }
            let requirement_ids = input
                .get("requirement_ids")
                .and_then(Value::as_array)
                .filter(|ids| !ids.is_empty())
                .ok_or_else(|| {
                    EvidenceCommandError::bad_request("run input requirement_ids must be non-empty")
                })?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        EvidenceCommandError::bad_request(
                            "run input requirement_ids must contain strings",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut canonical_ids = requirement_ids.clone();
            canonical_ids.sort();
            canonical_ids.dedup();
            if requirement_ids != canonical_ids {
                return Err(EvidenceCommandError::bad_request(
                    "run input requirement_ids must be sorted and unique",
                )
                .into());
            }
            if &requirement_ids != expected_requirement_ids {
                return Err(EvidenceCommandError::bad_request(
                    "run input requirement_ids do not equal the canonical target subset",
                )
                .into());
            }
            let capability = run["capability"].as_object().ok_or_else(|| {
                EvidenceCommandError::bad_request("run-index entry requires capability")
            })?;
            let capability_instance_id = input
                .get("capability_instance_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    EvidenceCommandError::bad_request(
                        "run input capability_instance_id is required",
                    )
                })?;
            if capability.get("instance_id").and_then(Value::as_str) != Some(capability_instance_id)
            {
                return Err(EvidenceCommandError::bad_request(
                    "run capability instance_id does not match run input",
                )
                .into());
            }
            let instance = self.load_capability_instance(capability_instance_id)?;
            if capability.get("manifest_id").and_then(Value::as_str)
                != Some(instance.manifest_id.as_str())
                || capability.get("manifest_digest").and_then(Value::as_str)
                    != Some(instance.manifest_digest.as_str())
                || capability.get("manifest_version").and_then(Value::as_str)
                    != Some(instance.adapter_version.as_str())
            {
                return Err(EvidenceCommandError::bad_request(
                    "run capability does not match the registered capability instance",
                )
                .into());
            }
            bindings.push(json!({
                "schema_version": "planr.evidence.execution-binding.v2",
                "run_index_digest": run_index_digest,
                "run_index": index,
                "obligation_id": obligation_id,
                "target": target,
                "requirement_ids": requirement_ids,
            }));
        }
        if seen.len() != expected.len() {
            return Err(EvidenceCommandError::bad_request(
                "run-index subsets do not form the exact authoritative union",
            )
            .into());
        }
        Ok(bindings)
    }

    fn host_capture_execution_subset(
        &self,
        value: &Value,
    ) -> Result<(Value, Value, VerificationCapabilityInstance)> {
        let run_index = value.get("run_index").ok_or_else(|| {
            EvidenceCommandError::bad_request("host capture requires a sealed run_index")
        })?;
        let (_, execution_bindings) = self.validate_sealed_run_index(run_index)?;
        let entry = value
            .get("run_index_entry")
            .and_then(Value::as_u64)
            .and_then(|entry| usize::try_from(entry).ok())
            .ok_or_else(|| {
                EvidenceCommandError::bad_request(
                    "host capture run_index_entry must be a non-negative integer",
                )
            })?;
        let run = run_index["runs"]
            .as_array()
            .and_then(|runs| runs.get(entry))
            .ok_or_else(|| {
                EvidenceCommandError::bad_request(
                    "host capture run_index_entry does not select a sealed run",
                )
            })?;
        let run_input = run.get("input").cloned().ok_or_else(|| {
            EvidenceCommandError::bad_request("sealed host capture run requires input")
        })?;
        let execution_binding = execution_bindings.get(entry).cloned().ok_or_else(|| {
            EvidenceCommandError::bad_request(
                "host capture run_index_entry has no execution binding",
            )
        })?;
        let instance_id = string_field(&run_input, "capability_instance_id")?;
        let instance = self.load_capability_instance(&instance_id)?;
        Ok((run_input, execution_binding, instance))
    }

    fn hermetic_reuse_binding(
        &self,
        input: HermeticReuseInput<'_>,
    ) -> Result<Option<HermeticReuseBinding>> {
        if std::env::var_os("PLANR_TEST_EVIDENCE_PRE_COMMIT_MUTATE_SOURCE_PATH").is_some() {
            return Ok(None);
        }
        let manifest_json: String = self.conn.query_row(
            "SELECT manifests.manifest_json
             FROM verification_capability_instances AS instances
             JOIN verification_capability_manifests AS manifests
               ON manifests.id = instances.manifest_id
              AND manifests.version = instances.manifest_version
             WHERE instances.id = ?1",
            params![input.instance.id.as_str()],
            |row| row.get(0),
        )?;
        let manifest: VerificationCapabilityManifest = serde_json::from_str(&manifest_json)?;
        let manifest_value = serde_json::to_value(&manifest)?;
        if !is_hermetic_reuse_candidate(
            &manifest,
            input.target,
            input.fixture_disclosure,
            input.env,
        )? {
            return Ok(None);
        }
        let snapshot = capture_repository_snapshot(&self.root)
            .map_err(|error| anyhow!("capturing hermetic reuse source: {error}"))?;
        let execution_contract_digest = crate::canonical_json::sha256_json_digest(
            &serde_json::to_value(input.execution_contract)?,
        )?;
        let toolchain_lock_digest = self.toolchain_lock_digest()?;
        let policy_digest = self
            .evidence_policy_document()?
            .ok_or_else(|| anyhow!("hermetic reuse requires current Evidence policy"))?
            .digest;
        let key = crate::canonical_json::sha256_json_digest(&json!({
            "schema_version": "planr.evidence.hermetic-reuse-key.v1",
            "obligation": input.obligation,
            "execution_contract_digest": execution_contract_digest,
            "source_tree_digest": snapshot.source.tree_digest,
            "toolchain_lock_digest": toolchain_lock_digest,
            "policy_digest": policy_digest,
            "capability_manifest": manifest_value,
            "capability_instance": input.instance,
            "target": input.target,
            "environment": input.environment,
            "execution_binding": input.execution_binding,
        }))?;
        Ok(Some(HermeticReuseBinding {
            key,
            execution_contract_digest,
            source_tree_digest: snapshot.source.tree_digest.as_str().to_string(),
            toolchain_lock_digest,
            execution_binding: input.execution_binding.clone(),
        }))
    }

    fn toolchain_lock_digest(&self) -> Result<String> {
        let mut bindings = Vec::new();
        for relative in [
            "Cargo.lock",
            "pnpm-lock.yaml",
            "package-lock.json",
            "yarn.lock",
            "bun.lock",
            "bun.lockb",
            "rust-toolchain",
            "rust-toolchain.toml",
            ".tool-versions",
        ] {
            let path = self.root.join(relative);
            if path.is_file() {
                bindings.push(json!({
                    "path": relative,
                    "digest": crate::canonical_json::sha256_prefixed_bytes(&fs::read(path)?),
                }));
            }
        }
        crate::canonical_json::sha256_json_digest(&json!(bindings))
    }

    fn reusable_hermetic_result(
        &self,
        project_id: &str,
        obligation_id: &str,
        binding: &HermeticReuseBinding,
    ) -> Result<Option<Value>> {
        let row = self
            .conn
            .query_row(
                "SELECT attempts.attempt_json, receipts.receipt_json, receipts.receipt_digest
                 FROM evidence_hermetic_check_cache AS cache
                 JOIN evidence_attempts AS attempts ON attempts.id = cache.attempt_id
                 JOIN evidence_receipts AS receipts ON receipts.id = cache.receipt_id
                 WHERE cache.reuse_key = ?1
                   AND cache.project_id = ?2
                   AND cache.obligation_id = ?3
                   AND attempts.attempt_status = 'passed'
                   AND receipts.receipt_status = 'trusted'
                   AND NOT EXISTS (
                     SELECT 1
                     FROM feature_run_evidence_invalidations AS invalidations
                     JOIN feature_runs AS runs ON runs.id = invalidations.run_id
                     JOIN json_each(invalidations.affected_evidence_ids_json) AS affected
                     WHERE runs.project_id = cache.project_id
                       AND affected.value = receipts.id
                   )
                 LIMIT 1",
                params![binding.key, project_id, obligation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(attempt, receipt, receipt_digest)| {
            let attempt = serde_json::from_str::<Value>(&attempt)?;
            if attempt
                .get("raw_result")
                .and_then(Value::as_object)
                .and_then(|raw_result| raw_result.get("execution_binding"))
                != Some(&binding.execution_binding)
            {
                bail!("hermetic reuse entry belongs to a different sealed execution subset");
            }
            Ok(json!({
                "attempt": attempt,
                "receipt": serde_json::from_str::<Value>(&receipt)?,
                "receipt_digest": receipt_digest,
                "verdict": "passed",
                "reused": true,
                "reuse_key": binding.key,
                "product_finding": Value::Null,
                "feature_run_lease": Value::Null,
            }))
        })
        .transpose()
    }

    fn store_hermetic_reuse(
        &self,
        project_id: &str,
        obligation_id: &str,
        attempt_id: &str,
        receipt_id: &str,
        binding: &HermeticReuseBinding,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO evidence_hermetic_check_cache(
               reuse_key, project_id, obligation_id, attempt_id, receipt_id,
               execution_contract_digest, source_tree_digest, toolchain_lock_digest, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
            params![
                binding.key,
                project_id,
                obligation_id,
                attempt_id,
                receipt_id,
                binding.execution_contract_digest,
                binding.source_tree_digest,
                binding.toolchain_lock_digest,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn evidence_run_value(&self, value: Value) -> Result<Value> {
        self.evidence_run_index_value(value)
    }

    fn evidence_run_single_value_with_routing(
        &self,
        value: Value,
        route_product_finding: bool,
        settle_terminal_exhaustion_atomically: bool,
        execution_binding: Value,
    ) -> Result<Value> {
        reject_trusted_receipt_input(&value)?;
        if value.get("feature_run_binding").is_some() {
            return Err(EvidenceCommandError::bad_request(
                "feature_run_binding is server-owned and must not be supplied",
            )
            .into());
        }
        if value.get("execution_binding").is_some() {
            return Err(EvidenceCommandError::bad_request(
                "execution_binding is server-owned and must not be supplied",
            )
            .into());
        }
        let project = self.default_project()?;
        let obligation_id = string_field(&value, "obligation_id")?;
        let instance_id = string_field(&value, "capability_instance_id")?;
        let obligation = self.load_proof_obligation(&obligation_id)?;
        let (obligation, target) =
            select_execution_binding_subset(obligation, &value, &execution_binding)
                .map_err(|error| EvidenceCommandError::bad_request(error.to_string()))?;
        let instance = self.load_capability_instance(&instance_id)?;
        let manifest = self.load_capability_manifest(&instance_id)?;
        let execution_contract: ProcessExecutionContract =
            serde_json::from_value(value.get("execution_contract").cloned().ok_or_else(|| {
                EvidenceCommandError::bad_request("sealed evidence run requires execution_contract")
            })?)?;
        let payload_json_schema = if execution_contract.payload_schema.schema_ref
            == "schema://planr.structured_observation_results.v1"
        {
            None
        } else {
            load_repository_observation_schema(
                &self.root,
                execution_contract.payload_schema.schema_ref.as_str(),
            )
            .map_err(|error| anyhow!("repository observation schema invalid: {error}"))?
        };
        let observation_payload_json_schemas = obligation
            .observations
            .iter()
            .filter_map(|observation| {
                observation
                    .payload_schema
                    .as_ref()
                    .map(|binding| (observation.id.as_str().to_string(), binding.schema_ref.clone()))
            })
            .map(|(requirement_id, schema_ref)| {
                let schema = load_repository_observation_schema(&self.root, &schema_ref)
                    .map_err(|error| {
                        anyhow!(
                            "repository observation schema invalid for {requirement_id} ({schema_ref}): {error}"
                        )
                    })?
                    .ok_or_else(|| {
                        anyhow!(
                            "repository observation schema missing for {requirement_id} ({schema_ref})"
                        )
                    })?;
                Ok((requirement_id, schema))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let environment: EnvironmentBinding =
            serde_json::from_value(value.get("environment").cloned().ok_or_else(|| {
                EvidenceCommandError::bad_request("sealed evidence run requires environment")
            })?)?;
        let fixture_disclosure: FixtureDisclosure = value
            .get("fixture_disclosure")
            .cloned()
            .ok_or_else(|| {
                EvidenceCommandError::bad_request("sealed evidence run requires fixture_disclosure")
            })
            .and_then(|value| {
                serde_json::from_value(value)
                    .map_err(|error| EvidenceCommandError::bad_request(error.to_string()))
            })?;
        let env = value
            .get("env")
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .map(|(key, value)| {
                        value
                            .as_str()
                            .map(|value| (key.clone(), value.to_string()))
                            .ok_or_else(|| anyhow!("env.{key} must be a string"))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()
            })
            .transpose()?
            .unwrap_or_default();
        if env.keys().any(|key| key.starts_with("PLANR_FIXTURE_"))
            && !fixture_disclosure.fixtures_used
        {
            anyhow::bail!(
                "PLANR_FIXTURE_* env controls require fixture_disclosure.fixtures_used=true"
            );
        }
        let retry_of = value
            .get("retry_of")
            .and_then(Value::as_str)
            .map(EvidenceId::parse)
            .transpose()?;
        let attempt_index = value
            .get("attempt_index")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let max_attempts = admitted_max_attempts(
            &manifest.repeatability,
            value.get("max_attempts").and_then(Value::as_u64),
        )?;
        let hermetic_reuse = if retry_of.is_none() && attempt_index == 0 {
            self.hermetic_reuse_binding(HermeticReuseInput {
                obligation: &obligation,
                instance: &instance,
                execution_contract: &execution_contract,
                target: &target,
                environment: &environment,
                fixture_disclosure: &fixture_disclosure,
                env: &env,
                execution_binding: &execution_binding,
            })?
        } else {
            None
        };
        let cancellation = crate::execution::CancellationToken::new();
        let lease =
            self.resolve_feature_run_evidence_lease(&project.id, obligation.plan_id.as_str())?;
        if let Some(binding) = hermetic_reuse.as_ref() {
            if let Some(lease) = lease.as_ref() {
                self.validate_feature_run_evidence_lease(&self.conn, lease)?;
            }
            if let Some(reused) =
                self.reusable_hermetic_result(&project.id, &obligation_id, binding)?
            {
                return Ok(reused);
            }
        }
        let guard = |conn: &rusqlite::Connection| -> Result<()> {
            let Some(lease) = lease.as_ref() else {
                return Ok(());
            };
            self.validate_feature_run_evidence_lease(conn, lease)
        };
        let one_shot_capability_instance_id = instance.id.as_str().to_string();
        let one_shot_retry_of = retry_of.as_ref().map(|id| id.as_str().to_string());
        let claim_one_shot = |conn: &rusqlite::Connection| -> Result<()> {
            let Some(lease) = lease.as_ref() else {
                return Ok(());
            };
            self.claim_non_repeatable_one_shot_in_transaction(
                conn,
                lease,
                obligation_id.as_str(),
                &one_shot_capability_instance_id,
                one_shot_retry_of.as_deref(),
                attempt_index,
                max_attempts,
                &manifest.repeatability,
            )
        };
        let terminal_settlement = RefCell::new(None);
        let settle_trusted_evidence = |conn: &rusqlite::Connection,
                                       attempt: &EvidenceAttempt,
                                       _receipt: &Value| {
            let verdict = evidence_run_verdict(attempt.status, &attempt.exit, &attempt.raw_result);
            if settle_terminal_exhaustion_atomically
                && should_settle_terminal_exhaustion(
                    &verdict,
                    lease.is_some(),
                    &manifest.repeatability,
                    attempt_index,
                    max_attempts,
                )
            {
                let lease = lease.as_ref().expect("predicate requires FeatureRun lease");
                let settlement = self.settle_exhausted_verification_attempt_in_transaction(
                    conn,
                    lease,
                    super::feature_run_evidence::VerificationAttemptExhaustion {
                        obligation_id: obligation_id.as_str(),
                        attempt_id: attempt.id.as_str(),
                        attempt_index,
                        max_attempts,
                        repeatability: &manifest.repeatability,
                    },
                )?;
                *terminal_settlement.borrow_mut() = Some(settlement);
            }
            Ok(())
        };
        let output = run_configured_process_adapter_guarded(
            &self.conn,
            ConfiguredProcessRunInput {
                repository_root: &self.root,
                project_id: &project.id,
                obligation,
                capability_instance: instance,
                execution_contract,
                payload_json_schema,
                observation_payload_json_schemas,
                target,
                environment,
                fixture_disclosure,
                env,
                retry_of,
                attempt_index,
                max_attempts,
                execution_binding,
                cancellation: &cancellation,
            },
            &guard,
            &claim_one_shot,
            &settle_trusted_evidence,
        );
        let output = output?;
        let verdict = evidence_run_verdict(
            output.attempt.status,
            &output.attempt.exit,
            &output.attempt.raw_result,
        );
        if verdict == "passed"
            && let Some(binding) = hermetic_reuse.as_ref()
            && let Some(receipt_id) = output.receipt_value["id"].as_str()
        {
            self.store_hermetic_reuse(
                &project.id,
                &obligation_id,
                output.attempt.id.as_str(),
                receipt_id,
                binding,
            )?;
        }
        let product_failed = output.receipt_value["proof_gaps"]
            .as_array()
            .is_some_and(|gaps| {
                gaps.iter().any(|gap| {
                    gap.as_str() == Some("product_failed")
                        || gap.get("reason").and_then(Value::as_str) == Some("product_failed")
                })
            });
        let product_finding =
            if product_failed && route_product_finding && terminal_settlement.borrow().is_none() {
                lease
                    .as_ref()
                    .map(|lease| {
                        self.route_evidence_product_finding_value(
                            &lease.run_id,
                            &lease.freeze_id,
                            obligation_id.as_str(),
                        )
                    })
                    .transpose()?
            } else {
                None
            };
        let terminal_exhaustion = terminal_settlement
            .into_inner()
            .map(|(item_id, run_id)| {
                Ok::<_, anyhow::Error>(json!({
                    "schema_version": "planr.verification-exhaustion.v1",
                    "status": "terminal_non_covering",
                    "item": item_id
                        .as_deref()
                        .map(|item_id| self.get_item(item_id))
                        .transpose()?,
                    "execution_state": self.canonical_execution_state_value(&run_id, None)?,
                }))
            })
            .transpose()?;
        Ok(json!({
            "attempt": output.attempt,
            "receipt": output.receipt_value,
            "receipt_digest": output.receipt_digest,
            "verdict": verdict,
            "reused": false,
            "reuse_key": hermetic_reuse.as_ref().map(|binding| binding.key.as_str()),
            "product_finding": product_finding,
            "feature_run_lease": lease,
            "terminal_exhaustion": terminal_exhaustion,
        }))
    }

    pub(crate) fn evidence_import_value(
        &self,
        value: Value,
        artifact_root: &Path,
    ) -> Result<Value> {
        reject_trusted_receipt_input(&value)?;
        let project = self.default_project()?;
        let root = if artifact_root.is_absolute() {
            artifact_root.to_path_buf()
        } else {
            self.root.join(artifact_root)
        };
        let record = parse_validated_artifact_import(
            value,
            &ValidatedArtifactImportRepository {
                conn: &self.conn,
                project_id: &project.id,
                artifact_root: &root,
            },
        )?;
        Ok(json!({
            "import": {
                "id": record.id,
                "digest": record.digest,
                "idempotent": record.idempotent,
                "proposal": proposal_value(&record.proposal),
            },
            "verdict": "valid",
        }))
    }

    pub(crate) fn evidence_host_capture_import_value(&self, value: Value) -> Result<Value> {
        self.evidence_host_capture_import_value_with_observed_run(value, None, None)
    }

    pub(crate) fn evidence_host_capture_run_value(&self, value: Value) -> Result<Value> {
        reject_trusted_receipt_input(&value)?;
        let object = value.as_object().ok_or_else(|| {
            EvidenceCommandError::bad_request("host capture run input must be a JSON object")
        })?;
        let allowed = BTreeSet::from([
            "schema_version",
            "run_index",
            "run_index_entry",
            "manifest_id",
            "experiment_id",
        ]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(EvidenceCommandError::bad_request(format!(
                "host capture run input has unknown fields: {}",
                unknown.join(",")
            ))
            .into());
        }
        let schema_version = string_field(&value, "schema_version")?;
        if schema_version != "planr.evidence.host_capture.run.v1" {
            return Err(EvidenceCommandError::bad_request(format!(
                "unsupported host capture run schema_version: {schema_version}"
            ))
            .into());
        }
        let (run_input, _, sealed_instance) = self.host_capture_execution_subset(&value)?;
        let obligation_id = string_field(&run_input, "obligation_id")?;
        if string_field(&value, "manifest_id")? != sealed_instance.manifest_id.as_str() {
            return Err(EvidenceCommandError::bad_request(
                "host capture manifest_id does not match the sealed capability",
            )
            .into());
        }
        let project = self.default_project()?;
        let obligation = self.load_proof_obligation(&obligation_id)?;
        let lease =
            self.resolve_feature_run_evidence_lease(&project.id, obligation.plan_id.as_str())?;
        let workflow_started_at = Instant::now();
        let observed_run =
            run_planr_observed_host_capture(self, &value, &obligation_id, |_timeout_ms| {
                if let Some(lease) = lease.as_ref() {
                    self.validate_feature_run_evidence_lease(&self.conn, lease)?;
                }
                Ok(true)
            })?;
        let workflow_timeout_ms = observed_run.workflow_timeout_ms;
        let mut import_input = json!({
            "schema_version": "planr.evidence.host_capture.import.v1",
            "run_index": value["run_index"].clone(),
            "run_index_entry": value["run_index_entry"].clone(),
            "import_root": observed_run.import_root.to_string_lossy(),
        });
        if let Some(experiment_id) = value.get("experiment_id").cloned() {
            import_input["experiment_id"] = experiment_id;
        }
        self.evidence_host_capture_import_value_with_observed_run(
            import_input,
            Some(observed_run),
            Some((workflow_started_at, workflow_timeout_ms)),
        )
    }

    fn evidence_host_capture_import_value_with_observed_run(
        &self,
        value: Value,
        observed_run: Option<ObservedHostCaptureRun>,
        workflow_deadline: Option<(Instant, u64)>,
    ) -> Result<Value> {
        reject_trusted_receipt_input(&value)?;
        let object = value.as_object().ok_or_else(|| {
            EvidenceCommandError::bad_request("host capture import input must be a JSON object")
        })?;
        let allowed = BTreeSet::from([
            "schema_version",
            "run_index",
            "run_index_entry",
            "import_root",
            "experiment_id",
        ]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(EvidenceCommandError::bad_request(format!(
                "host capture import input has unknown fields: {}",
                unknown.join(",")
            ))
            .into());
        }
        let schema_version = string_field(&value, "schema_version")?;
        if schema_version != "planr.evidence.host_capture.import.v1" {
            return Err(EvidenceCommandError::bad_request(format!(
                "unsupported host capture import schema_version: {schema_version}"
            ))
            .into());
        }
        let (run_input, execution_binding, sealed_instance) =
            self.host_capture_execution_subset(&value)?;
        let obligation_id = string_field(&run_input, "obligation_id")?;
        let project = self.default_project()?;
        let obligation = self.load_proof_obligation(&obligation_id)?;
        let (obligation, target) =
            select_execution_binding_subset(obligation, &run_input, &execution_binding)
                .map_err(|error| EvidenceCommandError::bad_request(error.to_string()))?;
        let lease =
            self.resolve_feature_run_evidence_lease(&project.id, obligation.plan_id.as_str())?;
        (|| -> Result<Value> {
            let import_root =
                resolve_evidence_input_path(&self.root, &string_field(&value, "import_root")?);
            let experiment_id = value
                .get("experiment_id")
                .and_then(Value::as_str)
                .unwrap_or("exp-chrome-browser-client");
            if experiment_id != "exp-chrome-browser-client" {
                return Err(EvidenceCommandError::bad_request(
                "only exp-chrome-browser-client host captures are supported by this import boundary",
            )
            .into());
            }

            let validator_timeout_ms = workflow_deadline
                .map(|(started, timeout_ms)| {
                    timeout_ms.saturating_sub(started.elapsed().as_millis() as u64)
                })
                .unwrap_or(HOST_CAPTURE_VALIDATOR_TIMEOUT_MS);
            let validated =
                validate_external_host_capture(&self.root, &import_root, validator_timeout_ms)?;
            let captures =
                crate::evidence::adapters::host::evaluate_phase1_host_fixture(&validated.root)
                    .map_err(|error| {
                        EvidenceCommandError::bad_request(format!(
                            "validated host capture bundle failed strict Evidence parsing: {error}"
                        ))
                    })?;
            let capture = captures
                .into_iter()
                .find(|capture| capture.experiment_id == experiment_id)
                .ok_or_else(|| {
                    EvidenceCommandError::bad_request(format!(
                        "validated host capture bundle is missing {experiment_id}"
                    ))
                })?;
            let adapter = if observed_run.is_some() {
                crate::evidence::adapters::codex::enable_chrome_browser_client_from_planr_observed_execution(capture.clone())?
            } else {
                crate::evidence::adapters::codex::enable_chrome_browser_client(capture.clone())?
            };
            if !adapter.trusted_adapter_enabled {
                return Err(EvidenceCommandError::bad_request(format!(
                    "host capture is not enabled: {}",
                    adapter.reason
                ))
                .into());
            }
            let captured_manifest = adapter
                .manifest
                .ok_or_else(|| anyhow!("enabled host capture missing manifest"))?;
            let captured_instance_value = adapter
                .instance
                .ok_or_else(|| anyhow!("enabled host capture missing instance"))?;
            let captured_instance: VerificationCapabilityInstance =
                serde_json::from_value(captured_instance_value)?;
            if let Some(run) = observed_run.as_ref() {
                for observation in &obligation.observations {
                    ensure_host_capture_run_manifest_supports_observation(run, observation)?;
                }
                if run.producer_instance.manifest_id != sealed_instance.manifest_id
                    || run.producer_instance.manifest_digest != sealed_instance.manifest_digest
                {
                    return Err(EvidenceCommandError::bad_request(
                        "observed host capture capability does not match the sealed capability",
                    )
                    .into());
                }
            } else if captured_instance.id != sealed_instance.id
                || captured_instance.manifest_id != sealed_instance.manifest_id
                || captured_instance.manifest_digest != sealed_instance.manifest_digest
                || captured_manifest.id != sealed_instance.manifest_id
            {
                return Err(EvidenceCommandError::bad_request(
                    "imported host capture capability does not match the sealed capability",
                )
                .into());
            }
            let evidence_manifest = self.load_capability_manifest(sealed_instance.id.as_str())?;
            let evidence_instance = sealed_instance;
            let evidence_instance_value = serde_json::to_value(&evidence_instance)?;
            let environment: EnvironmentBinding = serde_json::from_value(
                run_input.get("environment").cloned().ok_or_else(|| {
                    EvidenceCommandError::bad_request(
                        "sealed host capture run requires environment",
                    )
                })?,
            )?;
            let fixture_disclosure = observed_run
                .as_ref()
                .map(|run| run.fixture_disclosure.clone())
                .unwrap_or(FixtureDisclosure {
                    fixtures_used: false,
                    mocks_used: false,
                    fixture_refs: None,
                    mock_refs: None,
                });
            ensure_host_import_bindings(
                &obligation,
                &evidence_instance,
                &target,
                &environment,
                &fixture_disclosure,
            )?;
            ensure_host_capture_target_matches(&target, &capture.final_event_payload)?;
            for observation in &obligation.observations {
                ensure_expected_predicate_matches_capture(
                    &observation.expected,
                    &capture.final_event_payload,
                )?;
            }
            let valid_until = ensure_host_capture_fresh(&evidence_instance)?;

            let started_at = evidence_instance.captured_at.clone();
            let ended_at = evidence_instance.captured_at.clone();
            let attempt_id = host_capture_attempt_id(
                &obligation.id,
                &evidence_instance.id,
                &capture.raw_digest,
                &execution_binding,
            )?;
            let stdout_digest = crate::canonical_json::sha256_prefixed_bytes(&validated.stdout);
            let stderr_digest = crate::canonical_json::sha256_prefixed_bytes(&validated.stderr);
            let raw_result = json!({
                "schema_version": "planr.evidence.host_capture.result.v1",
                "validator": {
                    "path": validated.validator_path.to_string_lossy(),
                    "digest": validated.validator_digest.as_str(),
                },
                "normalized_root_digest": validated.normalized_root_digest,
                "summary": validated.summary,
                "capture": capture.final_event_payload,
                "execution_binding": execution_binding,
                "planr_observed_execution": observed_run.as_ref().map(|run| json!({
                    "manifest_id": run.producer_instance.manifest_id.as_str(),
                    "manifest_digest": run.producer_instance.manifest_digest.as_str(),
                    "instance_id": run.producer_instance.id.as_str(),
                    "provenance_source": run.provenance_source.as_str(),
                    "supported_observation_types": run.supported_observation_types,
                    "supported_artifacts": run.supported_artifacts,
                    "argv": run.argv,
                    "exit_code": run.exit_code,
                    "stdout_digest": run.stdout_digest.as_str(),
                    "stderr_digest": run.stderr_digest.as_str(),
                    "stdout_excerpt": run.stdout_excerpt.as_str(),
                    "stderr_excerpt": run.stderr_excerpt.as_str(),
                    "adapter_binding": run.adapter_binding.clone(),
                    "producer_fixture_disclosure_digest": run.fixture_disclosure_digest.as_str(),
                    "producer_fixture_disclosure": run.fixture_disclosure.clone(),
                })),
            });
            let raw_result_digest = crate::canonical_json::sha256_json_digest(&raw_result)?;
            let resolved_command = if let Some(run) = observed_run.as_ref() {
                json!({
                    "kind": "host_capture_run",
                    "manifest_id": run.producer_instance.manifest_id.as_str(),
                    "manifest_digest": run.producer_instance.manifest_digest.as_str(),
                    "instance_id": run.producer_instance.id.as_str(),
                    "provenance_source": run.provenance_source.as_str(),
                    "supported_observation_types": run.supported_observation_types,
                    "supported_artifacts": run.supported_artifacts,
                    "argv": run.argv,
                    "command": run.resolved_command.clone(),
                    "adapter_binding": run.adapter_binding.clone(),
                    "producer_fixture_disclosure_digest": run.fixture_disclosure_digest.as_str(),
                    "validator": {
                        "path": validated.validator_path.to_string_lossy(),
                        "digest": validated.validator_digest.as_str(),
                        "args": [
                            "capture",
                            "--out-dir",
                            validated.root.to_string_lossy(),
                            "--import-fixture-root",
                            import_root.to_string_lossy()
                        ],
                    },
                })
            } else {
                json!({
                    "kind": "host_capture_import",
                    "validator": {
                        "path": validated.validator_path.to_string_lossy(),
                        "digest": validated.validator_digest.as_str(),
                    },
                    "args": [
                        "capture",
                        "--out-dir",
                        validated.root.to_string_lossy(),
                        "--import-fixture-root",
                        import_root.to_string_lossy()
                    ],
                })
            };
            let artifact = capture
                .artifact_refs
                .iter()
                .find(|artifact| artifact.kind == "cdp-json-result")
                .ok_or_else(|| anyhow!("enabled host capture missing CDP artifact"))?;
            let attempt = EvidenceAttempt {
                id: EvidenceId::parse(attempt_id.clone()).map_err(|error| anyhow!(error))?,
                schema_version: crate::evidence::SchemaVersion::v1(),
                criterion_id: obligation.criterion_id.clone(),
                obligation_id: obligation.id.clone(),
                capability_instance_id: evidence_instance.id.clone(),
                started_at: started_at.clone(),
                ended_at: ended_at.clone(),
                status: AttemptStatus::Passed,
                resolved_command,
                exit: json!({"exit_code": 0, "signal": null, "error": null}),
                retry_lineage: json!({
                    "attempt_number": 1,
                    "max_attempts": 1,
                    "previous_attempt_ids": [],
                }),
                stdout_digest: Sha256Digest::parse(stdout_digest)
                    .map_err(|error| anyhow!(error))?,
                stderr_digest: Sha256Digest::parse(stderr_digest)
                    .map_err(|error| anyhow!(error))?,
                raw_result: raw_result.clone(),
                artifacts: vec![json!({
                    "id": artifact.id,
                    "kind": artifact.kind,
                    "digest": artifact.digest,
                    "uri": format!("host-capture://{}/{}", validated.normalized_root_digest, artifact.path),
                })],
                output_bounds: json!({
                    "stdout_bytes": validated.stdout.len(),
                    "stderr_bytes": validated.stderr.len(),
                    "truncated": false,
                }),
            };
            let repository_snapshot = capture_repository_snapshot(&self.root)
                .map_err(|err| anyhow!("capturing repository evidence snapshot: {err}"))?;
            let policy_binding = repository_snapshot.trusted_policy_binding()?;
            let instance_digest =
                crate::canonical_json::sha256_json_digest(&evidence_instance_value)?;
            let config_digest = crate::canonical_json::sha256_json_digest(&json!({
                "schema_version": schema_version,
                "obligation_id": obligation.id,
                "target": target,
                "environment": environment,
                "validated_capture_root_digest": validated.normalized_root_digest,
                "raw_digest": capture.raw_digest,
                "provenance_digest": capture.provenance_digest,
                    "artifact_digest": artifact.digest,
                "producer_capability": observed_run.as_ref().map(|run| json!({
                    "manifest_id": run.producer_instance.manifest_id.as_str(),
                    "manifest_digest": run.producer_instance.manifest_digest.as_str(),
                    "instance_id": run.producer_instance.id.as_str(),
                    "provenance_source": run.provenance_source.as_str(),
                    "supported_observation_types": run.supported_observation_types,
                    "supported_artifacts": run.supported_artifacts,
                    "adapter_binding": run.adapter_binding.clone(),
                    "producer_fixture_disclosure_digest": run.fixture_disclosure_digest.as_str(),
                })),
                "execution_binding": execution_binding,
                "fixture_disclosure": fixture_disclosure,
            }))?;
            let receipt = build_trusted_receipt(TrustedReceiptInput {
                id: EvidenceId::parse(format!("erec-{}", short_digest(&attempt_id)))
                    .map_err(|error| anyhow!(error))?,
                criterion_id: obligation.criterion_id.clone(),
                obligation_id: obligation.id.clone(),
                source: repository_snapshot.source.clone(),
                target: target.clone(),
                environment: environment.clone(),
                vantage_point: VantagePoint {
                    kind: if observed_run.is_some() {
                        "host_capture_run".to_string()
                    } else {
                        "host_capture_import".to_string()
                    },
                    identity: observed_run
                        .as_ref()
                        .map(|run| format!("planr/{}", run.producer_instance.manifest_id.as_str()))
                        .unwrap_or_else(|| "codex/chrome-browser-client".to_string()),
                },
                capability: CapabilityBinding {
                    manifest_id: evidence_instance.manifest_id.clone(),
                    manifest_digest: evidence_instance.manifest_digest.clone(),
                    instance_id: evidence_instance.id.clone(),
                    instance_digest: Sha256Digest::parse(instance_digest)
                        .map_err(|error| anyhow!(error))?,
                },
                provenance: TrustedProvenance {
                    source: observed_run
                        .as_ref()
                        .map(|run| run.provenance_source)
                        .unwrap_or(crate::evidence::ProvenanceSourceKind::VerifiedHostEvent),
                    assigned_by: "planr".to_string(),
                    execution_id: attempt_id.clone(),
                    tool_call_id: None,
                },
                observations: obligation
                    .observations
                    .iter()
                    .map(|observation| {
                        Ok(ObservationResult {
                            requirement_id: observation.id.clone(),
                            observation_type: observation.observation_type.clone(),
                            outcome: AttemptStatus::Passed,
                            predicate: map_from_json_object(
                                observation.expected.clone(),
                                "observation.expected",
                            )?,
                            actual: map_from_json_object(
                                json!({
                                    "final_event": capture.final_event_payload,
                                    "artifact_digest": artifact.digest,
                                    "availability_reason": capture.availability_reason,
                                }),
                                "host capture actual",
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                attempt_ids: vec![attempt.id.clone()],
                retry_history: Vec::new(),
                artifacts: vec![ArtifactRef {
                    id: EvidenceId::parse(artifact.id.clone()).map_err(|error| anyhow!(error))?,
                    kind: artifact.kind.clone(),
                    digest: Sha256Digest::parse(artifact.digest.clone())
                        .map_err(|error| anyhow!(error))?,
                    uri: Some(format!(
                        "host-capture://{}/{}",
                        validated.normalized_root_digest, artifact.path
                    )),
                    extra: serde_json::Map::new(),
                }],
                raw_result: RawResultRef {
                    kind: "host_capability_observed_raw".to_string(),
                    digest: Sha256Digest::parse(raw_result_digest)
                        .map_err(|error| anyhow!(error))?,
                    artifact_id: None,
                    extra: serde_json::Map::new(),
                },
                config_digest: Sha256Digest::parse(config_digest.clone())
                    .map_err(|error| anyhow!(error))?,
                fixture_disclosure: fixture_disclosure.clone(),
                permissions: evidence_instance.permissions.clone(),
                sandbox: SandboxState {
                    mode: if observed_run.is_some() {
                        "host_capture_run".to_string()
                    } else {
                        "host_capture_import".to_string()
                    },
                    limits: SandboxLimits {
                        timeout_ms: 30000,
                        stdout_bytes: 1048576,
                        stderr_bytes: 1048576,
                    },
                },
                proof_gaps: Vec::new(),
                started_at,
                ended_at,
            })
            .map_err(|error| anyhow!(error))?;
            let receipt_value = serde_json::to_value(&receipt)?;
            let receipt_digest = string_ref_field(&receipt_value, "receipt_digest")?.to_string();
            let trusted_binding_value =
                crate::evidence::policy::trusted_receipt_binding_value(&receipt, policy_binding)
                    .map_err(|error| anyhow!(error))?;
            let mut registry =
                CapabilityRegistry::from_manifests_and_adapter_registrations(&self.root, [], &[]);
            let persistence = persist_trusted_evidence_atomically(
                &self.conn,
                |conn| {
                    if let Some(lease) = lease.as_ref() {
                        self.validate_feature_run_evidence_lease(conn, lease)?;
                    }
                    registry.store_verified_host_capture_instance_with_expiry(
                        conn,
                        evidence_manifest.clone(),
                        evidence_instance.clone(),
                        Some(&valid_until),
                    )?;
                    if std::env::var_os("PLANR_TEST_HOST_CAPTURE_FAIL_AFTER_CAPABILITY_STORE")
                        .is_some()
                    {
                        bail!("test fault after host capture capability store");
                    }
                    Ok(())
                },
                |conn| {
                    run_repository_snapshot_pre_commit_test_hook(&self.root)?;
                    let current = capture_repository_snapshot(&self.root).map_err(|error| {
                        anyhow!("checking host-capture repository snapshot: {error}")
                    })?;
                    if current != repository_snapshot {
                        bail!(
                            "stale_source: repository changed before trusted host-capture receipt commit"
                        );
                    }
                    if let Some(lease) = lease.as_ref() {
                        self.validate_feature_run_evidence_lease(conn, lease)?;
                    }
                    Ok(())
                },
                TrustedEvidencePersistenceInput {
                    project_id: &project.id,
                    obligation_id: &obligation.id,
                    attempt: &attempt,
                    execution_contract_digest: &config_digest,
                    environment_digest: evidence_instance.environment.digest.as_str(),
                    retry_predecessor_attempt_id: None,
                    receipt_value: &receipt_value,
                    receipt_digest: &receipt_digest,
                    trusted_binding_value: &trusted_binding_value,
                },
            );
            persistence?;
            Ok(json!({
                "attempt": attempt,
                "receipt": receipt_value,
                "receipt_digest": receipt_digest,
                "capability": evidence_instance,
                "verdict": "trusted",
                "feature_run_lease": lease,
            }))
        })()
    }

    pub(crate) fn evidence_attempts_value(
        &self,
        id: Option<&str>,
        obligation: Option<&str>,
    ) -> Result<Value> {
        let project = self.default_project()?;
        if let Some(id) = id {
            return Ok(json!({"attempt": self.evidence_attempt_record_value(id)?}));
        }
        let rows = if let Some(obligation) = obligation {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, capability_instance_id, attempt_status,
                        execution_contract_digest, resolved_command_json, environment_digest,
                        retry_predecessor_attempt_id, started_at, completed_at, exit_code,
                        stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
                 FROM evidence_attempts WHERE project_id = ?1 AND obligation_id = ?2 ORDER BY created_at, id",
                params![project.id, obligation],
                attempt_row,
            )?
        } else {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, capability_instance_id, attempt_status,
                        execution_contract_digest, resolved_command_json, environment_digest,
                        retry_predecessor_attempt_id, started_at, completed_at, exit_code,
                        stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
                 FROM evidence_attempts WHERE project_id = ?1 ORDER BY created_at, id LIMIT 100",
                params![project.id],
                attempt_row,
            )?
        };
        Ok(json!({"attempts": rows}))
    }

    pub(crate) fn evidence_receipts_value(
        &self,
        id: Option<&str>,
        obligation: Option<&str>,
    ) -> Result<Value> {
        let project = self.default_project()?;
        if let Some(id) = id {
            return Ok(json!({"receipt": self.evidence_receipt_record_value(id)?}));
        }
        let rows = if let Some(obligation) = obligation {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, attempt_id, receipt_status,
                        receipt_digest, trusted_binding_json, observations_json, provenance_json,
                        receipt_json, supersedes_receipt_id, created_at
                 FROM evidence_receipts WHERE project_id = ?1 AND obligation_id = ?2 ORDER BY created_at, id",
                params![project.id, obligation],
                receipt_row,
            )?
        } else {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, attempt_id, receipt_status,
                        receipt_digest, trusted_binding_json, observations_json, provenance_json,
                        receipt_json, supersedes_receipt_id, created_at
                 FROM evidence_receipts WHERE project_id = ?1 ORDER BY created_at, id LIMIT 100",
                params![project.id],
                receipt_row,
            )?
        };
        Ok(json!({"receipts": rows}))
    }

    pub(crate) fn evidence_coverage_value(
        &self,
        scope: EvidenceCoverageScope,
        id: &str,
    ) -> Result<Value> {
        if matches!(scope, EvidenceCoverageScope::Plan) {
            match self.plan_evidence_authority(id)? {
                super::proof::PlanEvidenceAuthority::BindingActive => {}
                super::proof::PlanEvidenceAuthority::BindingUnsatisfied => {
                    let proof = self.proof_status_for_plan(id)?;
                    return Ok(json!({
                        "coverage": null,
                        "coverage_id": null,
                        "status": "unsatisfied",
                        "receipt_digests": [],
                        "waiver_digests": [],
                        "receipt_lineage": [],
                        "verdict": "unsatisfied",
                        "authority": "binding_unsatisfied",
                        "proof": proof,
                    }));
                }
                super::proof::PlanEvidenceAuthority::NonBinding => {
                    return Err(EvidenceCommandError::bad_request(
                        "nonbinding plans have no binding Evidence coverage; use the source-frozen final-review route",
                    )
                    .into());
                }
            }
        }
        let project = self.default_project()?;
        let evaluated_at = timestamp()?;
        let coverage = match scope {
            EvidenceCoverageScope::Obligation => {
                evaluate_obligation_coverage(&self.conn, &project.id, id, &evaluated_at)
            }
            EvidenceCoverageScope::Criterion => {
                evaluate_criterion_coverage(&self.conn, &project.id, id, &evaluated_at)
            }
            EvidenceCoverageScope::Item => {
                evaluate_item_coverage(&self.conn, &project.id, id, &evaluated_at)
            }
            EvidenceCoverageScope::Plan => {
                evaluate_plan_coverage(&self.conn, &project.id, id, &evaluated_at)
            }
        }
        .map_err(|err| anyhow!("{err}"))?;
        let mut value = json!({
            "coverage": coverage.verdict.clone(),
            "coverage_id": coverage.id.clone(),
            "status": coverage.status.as_str(),
            "receipt_digests": coverage.receipt_digests.clone(),
            "waiver_digests": coverage.waiver_digests.clone(),
            "receipt_lineage": coverage.receipt_lineage.clone(),
            "verdict": coverage.status.as_str(),
        });
        value["canonical_projection"] = canonical_coverage_projection(&value);
        if matches!(scope, EvidenceCoverageScope::Plan)
            && coverage.status.as_str() == "satisfied"
            && !coverage.receipt_digests.is_empty()
            && let Some(settlement) =
                self.settle_feature_run_after_plan_coverage(id, value.clone())?
        {
            value["feature_run_verification_settlement"] = settlement;
        }
        Ok(value)
    }

    pub(crate) fn current_plan_coverage_for_source_freeze(
        &self,
        project_id: &str,
        plan_id: &str,
        freeze: &SourceFreezeRecord,
    ) -> Result<CurrentPlanCoverageForSourceFreeze> {
        let evaluated_at = timestamp()?;
        let coverage = evaluate_plan_coverage(&self.conn, project_id, plan_id, &evaluated_at)
            .map_err(|error| anyhow!("{error}"))?;
        if coverage.status.as_str() != "satisfied" {
            return Ok(CurrentPlanCoverageForSourceFreeze::NeedsVerification(
                coverage,
            ));
        }
        match self
            .ensure_plan_coverage_matches_source_freeze(project_id, plan_id, freeze, &coverage)
        {
            Ok(()) => Ok(CurrentPlanCoverageForSourceFreeze::Satisfied(coverage)),
            Err(error)
                if error
                    .downcast_ref::<CoverageSourceFreezeMismatch>()
                    .is_some() =>
            {
                Ok(CurrentPlanCoverageForSourceFreeze::NeedsVerification(
                    coverage,
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn ensure_plan_coverage_matches_source_freeze(
        &self,
        project_id: &str,
        plan_id: &str,
        freeze: &SourceFreezeRecord,
        coverage: &CoverageEvaluation,
    ) -> Result<()> {
        if coverage.status.as_str() != "satisfied"
            || coverage.receipt_digests.is_empty()
            || !coverage.waiver_digests.is_empty()
            || coverage.verdict["scope"]["kind"].as_str() != Some("plan")
            || coverage.verdict["scope"]["id"].as_str() != Some(plan_id)
        {
            bail!("verification_coverage_requires_unwaived_satisfied_plan_coverage:{plan_id}");
        }
        let lineage = coverage
            .receipt_lineage
            .as_array()
            .ok_or_else(|| anyhow!("verification_coverage_receipt_lineage_invalid:{plan_id}"))?;
        let mut receipt_ids = BTreeSet::new();
        for observation in lineage {
            let covering = observation["covering_receipt_ids"]
                .as_array()
                .ok_or_else(|| {
                    anyhow!("verification_coverage_receipt_lineage_invalid:{plan_id}")
                })?;
            for receipt_id in covering {
                receipt_ids.insert(
                    receipt_id
                        .as_str()
                        .ok_or_else(|| {
                            anyhow!("verification_coverage_receipt_lineage_invalid:{plan_id}")
                        })?
                        .to_string(),
                );
            }
        }
        if receipt_ids.is_empty() {
            bail!("verification_coverage_receipt_lineage_empty:{plan_id}");
        }

        let mut receipt_digests = BTreeSet::new();
        for receipt_id in receipt_ids {
            let row: Option<(String, String, String)> = self
                .conn
                .query_row(
                    "SELECT receipts.receipt_digest, receipts.trusted_binding_json,
                            receipts.receipt_json
                     FROM evidence_receipts AS receipts
                     JOIN proof_obligations AS obligations
                       ON obligations.project_id = receipts.project_id
                      AND obligations.id = receipts.obligation_id
                     WHERE receipts.project_id = ?1 AND obligations.plan_id = ?2
                       AND receipts.id = ?3 AND receipts.receipt_status = 'trusted'",
                    params![project_id, plan_id, receipt_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let (receipt_digest, trusted_binding_json, receipt_json) = row.ok_or_else(|| {
                anyhow!("verification_coverage_receipt_missing:{plan_id}:{receipt_id}")
            })?;
            let receipt_value: Value = serde_json::from_str(&receipt_json)?;
            let receipt = EvidenceReceipt::from_trusted_value(receipt_value).map_err(|error| {
                anyhow!("verification_coverage_receipt_invalid:{receipt_id}:{error}")
            })?;
            if receipt.receipt_digest().as_str() != receipt_digest {
                bail!("verification_coverage_receipt_digest_mismatch:{receipt_id}");
            }
            let binding = parse_trusted_receipt_binding(&trusted_binding_json, &receipt).map_err(
                |error| anyhow!("verification_coverage_binding_invalid:{receipt_id}:{error}"),
            )?;
            if binding.source.revision != freeze.source_revision
                || binding.source.tree_digest.as_str() != freeze.source_digest
            {
                return Err(CoverageSourceFreezeMismatch {
                    freeze_id: freeze.id.clone(),
                    receipt_id,
                }
                .into());
            }
            receipt_digests.insert(receipt_digest);
        }
        let expected_digests = coverage
            .receipt_digests
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if receipt_digests != expected_digests {
            bail!("verification_coverage_receipt_set_mismatch:{plan_id}");
        }
        Ok(())
    }

    fn settle_feature_run_after_plan_coverage(
        &self,
        plan_id: &str,
        coverage_binding: Value,
    ) -> Result<Option<Value>> {
        let current_worker = crate::util::worker_id();
        if let Some(settlement) = self.settle_review_finding_reverification(
            plan_id,
            coverage_binding.clone(),
            &current_worker,
        )? {
            return Ok(Some(settlement));
        }
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT settle_feature_run_verification")?;
        let result = (|| -> Result<Option<Value>> {
            let project = self.default_project()?;
            let plan = self.get_plan(plan_id)?;
            let repository =
                super::repository::execution_run::ExecutionRunRepository::new(&self.conn);
            let Some(persisted) = repository.active_feature_run_for_plan(&project.id, plan_id)?
            else {
                return Ok(None);
            };
            if persisted.run.phase != FeatureRunPhase::Verification {
                return Ok(None);
            }
            let freeze = repository
                .active_source_freeze(&persisted.run.id)?
                .ok_or_else(|| {
                    anyhow!("verification_coverage_requires_active_source_freeze:{plan_id}")
                })?;
            let snapshot = capture_repository_snapshot(&self.root).map_err(|error| {
                anyhow!("checking verification coverage source freeze: {error}")
            })?;
            if snapshot.source.revision != freeze.source_revision
                || snapshot.source.tree_digest.as_str() != freeze.source_digest
            {
                bail!("verification_coverage_source_freeze_stale:{}", freeze.id);
            }

            let evaluated_at = timestamp()?;
            let locked_coverage =
                evaluate_plan_coverage(&self.conn, &project.id, plan_id, &evaluated_at)
                    .map_err(|error| anyhow!("{error}"))?;
            self.ensure_plan_coverage_matches_source_freeze(
                &project.id,
                plan_id,
                &freeze,
                &locked_coverage,
            )?;
            let mut coverage_binding = json!({
                "coverage": locked_coverage.verdict.clone(),
                "coverage_id": locked_coverage.id.clone(),
                "status": locked_coverage.status.as_str(),
                "receipt_digests": locked_coverage.receipt_digests.clone(),
                "waiver_digests": locked_coverage.waiver_digests.clone(),
                "receipt_lineage": locked_coverage.receipt_lineage.clone(),
                "verdict": locked_coverage.status.as_str(),
            });
            coverage_binding["canonical_projection"] =
                canonical_coverage_projection(&coverage_binding);
            let verifier = persisted
                .run
                .role_owners
                .iter()
                .find(|owner| owner.role == RunRole::Verifier)
                .ok_or_else(|| {
                    anyhow!(
                        "verification_coverage_missing_verifier:{}",
                        persisted.run.id
                    )
                })?;
            if verifier.worker_id != current_worker {
                bail!(
                    "verification_coverage_requires_verifier_lease:{}",
                    verifier.worker_id
                );
            }

            let verification_items = self
                .conn
                .prepare(
                    "SELECT id, status, worker_id FROM items
                     WHERE project_id = ?1 AND plan_path = ?2 AND work_type = 'verification'
                       AND status IN ('ready','picked','running')
                     ORDER BY priority DESC, created_at, id",
                )?
                .query_map(params![project.id, plan.path], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if verification_items
                .iter()
                .any(|(_, status, _)| status == "ready")
            {
                bail!("verification_coverage_requires_verification_item_lease:{plan_id}");
            }
            let active_items = verification_items
                .into_iter()
                .filter(|(_, status, _)| matches!(status.as_str(), "picked" | "running"))
                .collect::<Vec<_>>();
            if active_items.len() > 1 {
                bail!(
                    "verification_coverage_ambiguous_active_verification_items:{plan_id}:{}",
                    active_items.len()
                );
            }
            let item_id = active_items
                .first()
                .map(|(item_id, _, item_worker)| {
                    if item_worker.as_deref() != Some(current_worker.as_str()) {
                        bail!("verification_coverage_stale_item_owner:{item_id}");
                    }
                    Ok(item_id.clone())
                })
                .transpose()?;
            let log_id = if let Some(item_id) = item_id.as_deref() {
                let command = format!("planr evidence coverage --scope plan --id {plan_id}");
                let log_id = self.add_log_entry(super::flow::LogInput {
                    item_id,
                    kind: "completion",
                    summary: "canonical plan Evidence coverage settled the FeatureRun verification lifecycle",
                    files: &[],
                    commands: &[command],
                    tests: &[],
                    source: Some("evidence.coverage"),
                    profile: None,
                    route_observation: None,
                })?;
                let changed = self.conn.execute(
                    "UPDATE items
                     SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now')
                     WHERE id = ?1 AND status IN ('picked','running') AND worker_id = ?2",
                    params![item_id, current_worker],
                )?;
                if changed != 1 {
                    bail!("verification_item_close_conflict:{item_id}");
                }
                self.promote_ready()?;
                Some(log_id)
            } else {
                None
            };
            let next_ordinary_item_id =
                self.peek_next_ready_item_filtered(&super::lease::PickFilter {
                    exclude: None,
                    work_type: None,
                    plan_path: Some(plan.path.as_str()),
                    ordinary_implementation: true,
                })?;
            let has_open_ordinary_outcomes =
                !repository.open_ordinary_outcome_ids(plan_id)?.is_empty();
            self.reconcile_active_phase_wall(
                &persisted.run.id,
                crate::usage_policy::BudgetPhase::Verification,
            )?;
            let coverage_id = coverage_binding["coverage_id"].as_str().unwrap_or(plan_id);
            let (phase, next_action) = if has_open_ordinary_outcomes {
                let maker_worker_id = self.conn.query_row(
                    "SELECT worker_id FROM feature_run_role_leases
                     WHERE run_id = ?1 AND role = 'maker'
                     ORDER BY lease_generation DESC LIMIT 1",
                    [&persisted.run.id],
                    |row| row.get::<_, String>(0),
                )?;
                let maker_generation = self.conn.query_row(
                    "SELECT COALESCE(MAX(lease_generation), 0) + 1
                     FROM feature_run_role_leases WHERE run_id = ?1 AND role = 'maker'",
                    [&persisted.run.id],
                    |row| row.get::<_, u64>(0),
                )?;
                let mut implementation = apply_phase_transition(
                    &persisted.run,
                    &PhaseTransition {
                        to: FeatureRunPhase::Implementation,
                        cause: PhaseTransitionCause::VerificationPassed,
                        reference: format!("evidence_coverage:{coverage_id}"),
                        owner: Some(RoleOwner {
                            role: RunRole::Maker,
                            worker_id: maker_worker_id.clone(),
                            lease_generation: maker_generation,
                        }),
                    },
                )
                .map_err(|violation| {
                    anyhow!("verification_continuation_transition:{violation:?}")
                })?;
                let batch = ExecutionBatch {
                    id: short_id("batch"),
                    run_id: persisted.run.id.clone(),
                    maker_worker_id,
                    status: ExecutionBatchStatus::Active,
                    settled_outcome_ids: Vec::new(),
                    replacement: None,
                };
                implementation.active_batch_id = Some(batch.id.clone());
                implementation.batch_outcome_count = 0;
                repository.save_feature_run_with_new_batch(
                    &implementation,
                    persisted.revision,
                    &batch,
                )?;
                (
                    FeatureRunPhase::Implementation,
                    format!("planr pick --plan {plan_id} --json"),
                )
            } else {
                let source_frozen = apply_phase_transition(
                    &persisted.run,
                    &PhaseTransition {
                        to: FeatureRunPhase::SourceFrozen,
                        cause: PhaseTransitionCause::VerificationPassed,
                        reference: format!("evidence_coverage:{coverage_id}"),
                        owner: None,
                    },
                )
                .map_err(|violation| {
                    anyhow!("verification_final_review_transition:{violation:?}")
                })?;
                repository.save_feature_run(&source_frozen, persisted.revision)?;
                (
                    FeatureRunPhase::SourceFrozen,
                    format!("planr plan final-review {plan_id}"),
                )
            };
            self.record_event(
                "feature_run_verification_settled",
                item_id.as_deref(),
                json!({
                    "plan_id": plan_id,
                    "run_id": persisted.run.id,
                    "freeze_id": freeze.id,
                    "item_id": item_id.clone(),
                    "log_id": log_id.clone(),
                    "coverage": coverage_binding.clone(),
                    "phase": phase,
                    "next_ordinary_item_id": next_ordinary_item_id,
                }),
            )?;
            Ok(Some(json!({
                "item_id": item_id.clone(),
                "item_status": item_id.as_ref().map(|_| "closed"),
                "status": "settled",
                "log_id": log_id,
                "coverage": coverage_binding,
                "phase": phase,
                "next_ordinary_item_id": next_ordinary_item_id,
                "next_action": next_action,
            })))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE settle_feature_run_verification; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO settle_feature_run_verification; RELEASE settle_feature_run_verification; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    fn settle_review_finding_reverification(
        &self,
        plan_id: &str,
        coverage_binding: Value,
        verifier_worker_id: &str,
    ) -> Result<Option<Value>> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT settle_review_finding_reverification")?;
        let result = (|| -> Result<Option<Value>> {
            let project = self.default_project()?;
            let repository = ExecutionRunRepository::new(&self.conn);
            let Some(current) = repository.active_feature_run_for_plan(&project.id, plan_id)?
            else {
                return Ok(None);
            };
            let run_id = current.run.id.as_str();
            let Some(gate) = repository
                .review_gates_for_run(run_id, false)?
                .into_iter()
                .find(|gate| gate.kind == ReviewGateKind::FinalProduct)
            else {
                return Ok(None);
            };
            let findings = repository.findings(&gate.id)?;
            if findings.is_empty() {
                return Ok(None);
            }
            if findings
                .iter()
                .any(|finding| finding.status != FindingStatus::Resolved)
            {
                if current.run.phase == FeatureRunPhase::Verification {
                    bail!("review_reverification_findings_not_resolved:{}", gate.id);
                }
                return Ok(None);
            }
            if !matches!(
                current.run.phase,
                FeatureRunPhase::Verification | FeatureRunPhase::SourceFrozen
            ) {
                return Ok(None);
            }

            let evaluated_at = timestamp()?;
            let coverage = evaluate_plan_coverage(&self.conn, &project.id, plan_id, &evaluated_at)
                .map_err(|error| anyhow!("{error}"))?;
            let mut locked_coverage = json!({
                "coverage": coverage.verdict,
                "coverage_id": coverage.id,
                "status": coverage.status.as_str(),
                "receipt_digests": coverage.receipt_digests,
                "waiver_digests": coverage.waiver_digests,
                "receipt_lineage": coverage.receipt_lineage,
                "verdict": coverage.status.as_str(),
            });
            locked_coverage["canonical_projection"] =
                canonical_coverage_projection(&locked_coverage);
            for field in [
                "coverage_id",
                "status",
                "receipt_digests",
                "waiver_digests",
                "receipt_lineage",
            ] {
                if coverage_binding[field] != locked_coverage[field] {
                    bail!("review_reverification_coverage_changed:{plan_id}:{field}");
                }
            }
            if locked_coverage["status"] != "satisfied"
                || locked_coverage["receipt_digests"]
                    .as_array()
                    .is_none_or(Vec::is_empty)
                || locked_coverage["waiver_digests"]
                    .as_array()
                    .is_some_and(|values| !values.is_empty())
            {
                bail!("review_reverification_requires_unwaived_satisfied_coverage:{plan_id}");
            }

            let freeze = repository
                .active_source_freeze(run_id)?
                .ok_or_else(|| anyhow!("review_reverification_freeze_missing:{run_id}"))?;
            let binding = ReviewSourceBindingRecord {
                gate_id: gate.id.clone(),
                freeze_id: freeze.id.clone(),
                source_revision: freeze.source_revision.clone(),
                source_digest: freeze.source_digest.clone(),
                receipt_lineage: locked_coverage["receipt_lineage"].clone(),
            };

            if gate.status == ReviewGateStatus::Pending {
                if current.run.phase != FeatureRunPhase::SourceFrozen {
                    bail!(
                        "review_reverification_idempotence_phase_conflict:{}",
                        gate.id
                    );
                }
                let stored = repository
                    .review_source_binding(&gate.id)?
                    .ok_or_else(|| anyhow!("review_reverification_binding_missing:{}", gate.id))?;
                if stored != binding {
                    bail!("review_reverification_idempotence_conflict:{}", gate.id);
                }
                return Ok(Some(json!({
                    "mode": "review_finding_reverification",
                    "review_gate_id": gate.id,
                    "status": "already_settled",
                    "source_freeze": freeze,
                    "coverage": locked_coverage,
                    "next_action": format!("planr pick --plan {plan_id} --work-type review --json"),
                })));
            }
            if gate.status != ReviewGateStatus::ChangesRequested {
                bail!("review_reverification_gate_not_repairing:{}", gate.id);
            }
            if current.run.phase != FeatureRunPhase::Verification {
                bail!("review_reverification_phase_conflict:{run_id}");
            }
            let verifier = current
                .run
                .role_owners
                .iter()
                .find(|owner| owner.role == RunRole::Verifier)
                .ok_or_else(|| anyhow!("review_reverification_verifier_missing:{run_id}"))?;
            if verifier.worker_id != verifier_worker_id {
                bail!("review_reverification_verifier_conflict:{run_id}");
            }
            let snapshot = capture_repository_snapshot(&self.root)
                .map_err(|error| anyhow!("review_reverification_source_capture:{error}"))?;
            if snapshot.source.revision != freeze.source_revision
                || snapshot.source.tree_digest.as_str() != freeze.source_digest
            {
                bail!("review_reverification_source_stale:{}", freeze.id);
            }
            repository.rebind_review_gate_source(&binding)?;
            repository.set_review_gate_status(
                &gate.id,
                ReviewGateStatus::ChangesRequested,
                ReviewGateStatus::Pending,
            )?;
            self.reconcile_active_phase_wall(run_id, crate::usage_policy::BudgetPhase::Repair)?;
            self.reconcile_active_phase_wall(
                run_id,
                crate::usage_policy::BudgetPhase::Verification,
            )?;
            let review_ready = apply_phase_transition(
                &current.run,
                &PhaseTransition {
                    to: FeatureRunPhase::SourceFrozen,
                    cause: PhaseTransitionCause::VerificationPassed,
                    reference: format!("review_gate:{}", gate.id),
                    owner: None,
                },
            )
            .map_err(|violation| anyhow!("review_reverification_transition:{violation:?}"))?;
            repository.save_feature_run(&review_ready, current.revision)?;
            Ok(Some(json!({
                "mode": "review_finding_reverification",
                "review_gate_id": gate.id,
                "status": "settled",
                "source_freeze": freeze,
                "coverage": locked_coverage,
                "next_action": format!("planr pick --plan {plan_id} --work-type review --json"),
            })))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE settle_review_finding_reverification; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK TO settle_review_finding_reverification; RELEASE settle_review_finding_reverification; ROLLBACK");
                Err(error)
            }
        }
    }

    pub(crate) fn evidence_plan_criterion_coverages_value(
        &self,
        plan_id: &str,
    ) -> Result<Vec<Value>> {
        let project = self.default_project()?;
        let evaluated_at = timestamp()?;
        evaluate_plan_criterion_coverages(&self.conn, &project.id, plan_id, &evaluated_at)
            .map_err(|err| anyhow!("{err}"))?
            .into_iter()
            .map(|coverage| {
                let mut value = json!({
                    "coverage": coverage.verdict,
                    "coverage_id": coverage.id,
                    "status": coverage.status.as_str(),
                    "receipt_digests": coverage.receipt_digests,
                    "waiver_digests": coverage.waiver_digests,
                    "verdict": coverage.status.as_str(),
                });
                value["canonical_projection"] = canonical_coverage_projection(&value);
                Ok(value)
            })
            .collect()
    }

    pub(crate) fn evidence_item_criterion_coverages_value(
        &self,
        item_id: &str,
    ) -> Result<Vec<Value>> {
        let project = self.default_project()?;
        let evaluated_at = timestamp()?;
        evaluate_item_criterion_coverages(&self.conn, &project.id, item_id, &evaluated_at)
            .map_err(|err| anyhow!("{err}"))?
            .into_iter()
            .map(|coverage| {
                let mut value = json!({
                    "coverage": coverage.verdict,
                    "coverage_id": coverage.id,
                    "status": coverage.status.as_str(),
                    "receipt_digests": coverage.receipt_digests,
                    "waiver_digests": coverage.waiver_digests,
                    "verdict": coverage.status.as_str(),
                });
                value["canonical_projection"] = canonical_coverage_projection(&value);
                Ok(value)
            })
            .collect()
    }

    pub(crate) fn evidence_explain_value(
        &self,
        scope: EvidenceCoverageScope,
        id: &str,
    ) -> Result<Value> {
        let coverage = self.evidence_coverage_value(scope, id)?;
        let project = self.default_project()?;
        let obligation_ids =
            authoritative_obligation_ids_for_scope(&self.conn, &project.id, scope.as_str(), id)
                .map_err(|err| anyhow!("{err}"))?;
        Ok(json!({
            "explain": {
                "coverage": coverage,
                "scope": {"kind": scope.as_str(), "id": id},
                "obligation_ids": obligation_ids,
                "attempts": self.evidence_attempts_for_obligations(&project.id, &obligation_ids)?,
                "receipts": self.evidence_receipts_for_obligations(&project.id, &obligation_ids)?,
                "policy": self.evidence_policy_value()?,
                "repository_snapshot": capture_repository_snapshot(&self.root)
                    .ok()
                    .map(|snapshot| serde_json::to_value(snapshot.source).unwrap_or(Value::Null)),
            },
            "verdict": coverage["verdict"],
        }))
    }

    pub(crate) fn evidence_doctor_value(&self) -> Result<Value> {
        let policy = self.evidence_policy_doctor_value();
        let project = self.default_project()?;
        let obligations: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM proof_obligations WHERE project_id = ?1",
            params![project.id],
            |row| row.get(0),
        )?;
        let capability_instances: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM verification_capability_instances",
            [],
            |row| row.get(0),
        )?;
        let attempts: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_attempts WHERE project_id = ?1",
            params![project.id],
            |row| row.get(0),
        )?;
        let receipts: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM evidence_receipts WHERE project_id = ?1",
            params![project.id],
            |row| row.get(0),
        )?;
        let registry_diagnostics = policy["registry"]["diagnostics"]
            .as_array()
            .map_or(0, Vec::len);
        let policy_diagnostics = policy["diagnostics"].as_array().map_or(0, Vec::len);
        let registered_capabilities = policy["registry"]["registered_capabilities"]
            .as_u64()
            .unwrap_or(0);
        let available_capabilities = policy["registry"]["available_capabilities"]
            .as_u64()
            .unwrap_or(0);
        let policy_state = policy["state"].as_str().unwrap_or("malformed_policy");
        Ok(json!({
            "status": if policy_state == "ready" && policy["status"] == "valid" && registered_capabilities > 0 && available_capabilities == registered_capabilities && registry_diagnostics == 0 && policy_diagnostics == 0 { "ok" } else { "warning" },
            "policy": policy,
            "storage": {
                "proof_obligations": obligations,
                "capability_instances": capability_instances,
                "available_capability_instances": available_capabilities,
                "attempts": attempts,
                "receipts": receipts,
            },
            "observe_only": true,
        }))
    }

    fn evidence_obligation_record_value(&self, id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT id, project_id, plan_id, item_id, criterion_id, title, binding,
                        observation_requirements_json, fixture_policy_json, freshness_policy_json,
                        assurance_policy_json, retry_aggregation, policy_digest, config_digest, source_digest,
                        supersedes_obligation_id, obligation_version, created_at, obligation_shape
                 FROM proof_obligations WHERE project_id = ?1 AND id = ?2",
                params![self.default_project()?.id, id],
                obligation_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("evidence obligation not found: {id}"))
    }

    fn load_proof_obligation(&self, id: &str) -> Result<ProofObligation> {
        let value = self.evidence_obligation_record_value(id)?;
        if value["obligation_shape"] != "semantic_v1" {
            bail!("legacy proof obligation is historical and cannot be executed: {id}");
        }
        serde_json::from_value(json!({
            "id": value["id"],
            "schema_version": "evidence.contract.v1",
            "criterion_id": value["criterion_id"],
            "plan_id": value["plan_id"],
            "item_id": value["item_id"],
            "title": value["title"],
            "binding": value["binding"],
            "observations": value["observations"],
            "fixture_policy": value["fixture_policy"],
            "freshness_policy": value["freshness_policy"],
            "assurance_policy": value["assurance_policy"],
            "supersedes": value["supersedes_obligation_id"],
        }))
        .context("decoding stored proof obligation")
    }

    fn load_capability_instance(&self, id: &str) -> Result<VerificationCapabilityInstance> {
        let (manifest_id, manifest_version, manifest_digest, snapshot): (
            String,
            String,
            String,
            String,
        ) = self.conn.query_row(
            "SELECT manifest_id, manifest_version, manifest_digest, capability_snapshot_json
             FROM verification_capability_instances WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let instance: VerificationCapabilityInstance =
            serde_json::from_str(&snapshot).context("decoding stored capability instance")?;
        if instance.manifest_id.as_str() != manifest_id
            || instance.adapter_version != manifest_version
            || instance.manifest_digest.as_str() != manifest_digest
        {
            bail!(
                "capability instance does not match persisted registration: row {manifest_id}@{manifest_version} {manifest_digest}, snapshot {}@{} {}",
                instance.manifest_id.as_str(),
                instance.adapter_version,
                instance.manifest_digest.as_str()
            );
        }
        Ok(instance)
    }

    fn load_capability_manifest(
        &self,
        instance_id: &str,
    ) -> Result<VerificationCapabilityManifest> {
        let manifest_json: String = self.conn.query_row(
            "SELECT m.manifest_json
             FROM verification_capability_instances i
             JOIN verification_capability_manifests m
               ON m.id = i.manifest_id AND m.version = i.manifest_version
             WHERE i.id = ?1",
            params![instance_id],
            |row| row.get(0),
        )?;
        serde_json::from_str(&manifest_json).context("decoding capability manifest")
    }

    fn evidence_attempt_record_value(&self, id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT id, project_id, obligation_id, capability_instance_id, attempt_status,
                        execution_contract_digest, resolved_command_json, environment_digest,
                        retry_predecessor_attempt_id, started_at, completed_at, exit_code,
                        stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
                 FROM evidence_attempts WHERE project_id = ?1 AND id = ?2",
                params![self.default_project()?.id, id],
                attempt_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("evidence attempt not found: {id}"))
    }

    fn evidence_receipt_record_value(&self, id: &str) -> Result<Value> {
        self.conn
            .query_row(
                "SELECT id, project_id, obligation_id, attempt_id, receipt_status,
                        receipt_digest, trusted_binding_json, observations_json, provenance_json,
                        receipt_json, supersedes_receipt_id, created_at
                 FROM evidence_receipts WHERE project_id = ?1 AND id = ?2",
                params![self.default_project()?.id, id],
                receipt_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("evidence receipt not found: {id}"))
    }

    fn evidence_attempts_for_obligations(
        &self,
        project_id: &str,
        obligation_ids: &[String],
    ) -> Result<Vec<Value>> {
        let mut attempts = Vec::new();
        for obligation_id in obligation_ids {
            attempts.extend(
                self.evidence_attempts_value_for_project(
                    project_id,
                    None,
                    Some(obligation_id.as_str()),
                )?["attempts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        Ok(attempts)
    }

    fn evidence_receipts_for_obligations(
        &self,
        project_id: &str,
        obligation_ids: &[String],
    ) -> Result<Vec<Value>> {
        let mut receipts = Vec::new();
        for obligation_id in obligation_ids {
            receipts.extend(
                self.evidence_receipts_value_for_project(
                    project_id,
                    None,
                    Some(obligation_id.as_str()),
                )?["receipts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        Ok(receipts)
    }

    fn evidence_attempts_value_for_project(
        &self,
        project_id: &str,
        id: Option<&str>,
        obligation: Option<&str>,
    ) -> Result<Value> {
        if let Some(id) = id {
            return Ok(json!({"attempt": self.evidence_attempt_record_value(id)?}));
        }
        let rows = if let Some(obligation) = obligation {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, capability_instance_id, attempt_status,
                        execution_contract_digest, resolved_command_json, environment_digest,
                        retry_predecessor_attempt_id, started_at, completed_at, exit_code,
                        stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
                 FROM evidence_attempts WHERE project_id = ?1 AND obligation_id = ?2 ORDER BY created_at, id",
                params![project_id, obligation],
                attempt_row,
            )?
        } else {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, capability_instance_id, attempt_status,
                        execution_contract_digest, resolved_command_json, environment_digest,
                        retry_predecessor_attempt_id, started_at, completed_at, exit_code,
                        stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
                 FROM evidence_attempts WHERE project_id = ?1 ORDER BY created_at, id LIMIT 100",
                params![project_id],
                attempt_row,
            )?
        };
        Ok(json!({"attempts": rows}))
    }

    fn evidence_receipts_value_for_project(
        &self,
        project_id: &str,
        id: Option<&str>,
        obligation: Option<&str>,
    ) -> Result<Value> {
        if let Some(id) = id {
            return Ok(json!({"receipt": self.evidence_receipt_record_value(id)?}));
        }
        let rows = if let Some(obligation) = obligation {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, attempt_id, receipt_status,
                        receipt_digest, trusted_binding_json, observations_json, provenance_json,
                        receipt_json, supersedes_receipt_id, created_at
                 FROM evidence_receipts WHERE project_id = ?1 AND obligation_id = ?2 ORDER BY created_at, id",
                params![project_id, obligation],
                receipt_row,
            )?
        } else {
            query_json_rows(
                &self.conn,
                "SELECT id, project_id, obligation_id, attempt_id, receipt_status,
                        receipt_digest, trusted_binding_json, observations_json, provenance_json,
                        receipt_json, supersedes_receipt_id, created_at
                 FROM evidence_receipts WHERE project_id = ?1 ORDER BY created_at, id LIMIT 100",
                params![project_id],
                receipt_row,
            )?
        };
        Ok(json!({"receipts": rows}))
    }
}

fn canonical_target_partitions(
    observations: &[ObservationRequirement],
) -> Result<Vec<(Value, Vec<String>)>> {
    if observations.is_empty() {
        bail!("canonical Evidence obligation has no observations");
    }
    let mut partitions = BTreeMap::<String, (Value, Vec<String>)>::new();
    for observation in observations {
        let target_digest = crate::canonical_json::sha256_json_digest(&observation.target)?;
        let entry = partitions
            .entry(target_digest)
            .or_insert_with(|| (observation.target.clone(), Vec::new()));
        if entry.0 != observation.target {
            bail!("canonical target digest collision");
        }
        entry.1.push(observation.id.as_str().to_string());
    }
    let mut result = partitions.into_values().collect::<Vec<_>>();
    for (_, requirement_ids) in &mut result {
        requirement_ids.sort();
        requirement_ids.dedup();
        if requirement_ids.is_empty() {
            bail!("canonical target partition has no requirement_ids");
        }
    }
    Ok(result)
}

fn evidence_run_verdict(status: AttemptStatus, exit: &Value, raw_result: &Value) -> String {
    exit.get("error")
        .and_then(Value::as_str)
        .filter(|error| {
            *error == "verifier_failed"
                && (raw_result.get("ordinary_observation_error").is_some()
                    || raw_result.get("structured_observation_error").is_some())
        })
        .unwrap_or_else(|| status.as_str())
        .to_string()
}

fn should_settle_terminal_exhaustion(
    verdict: &str,
    has_feature_run_lease: bool,
    repeatability: &str,
    attempt_index: u32,
    max_attempts: u32,
) -> bool {
    verdict != "passed"
        && has_feature_run_lease
        && repeatability == "non_repeatable_one_shot"
        && max_attempts > 0
        && attempt_index.checked_add(1) == Some(max_attempts)
}

fn admitted_max_attempts(repeatability: &str, declared: Option<u64>) -> Result<u32> {
    let declared = declared
        .map(u32::try_from)
        .transpose()
        .context("max_attempts does not fit u32")?;
    if repeatability == "non_repeatable_one_shot" {
        if declared.is_some_and(|max_attempts| max_attempts != 1) {
            bail!("non_repeatable_one_shot requires max_attempts = 1");
        }
        Ok(1)
    } else {
        Ok(declared.unwrap_or(3))
    }
}

fn evidence_run_index_verdict(results: &[Value]) -> &'static str {
    let mut verdict = "passed";
    for result in results {
        match result["verdict"].as_str() {
            Some("passed" | "trusted" | "valid" | "satisfied" | "waived") => {}
            Some("timed_out") => return "timed_out",
            Some("aborted") => return "aborted",
            Some("unavailable") => return "unavailable",
            Some("inconclusive") if verdict == "passed" => verdict = "inconclusive",
            Some("failed" | "verifier_failed" | "skipped") | None => verdict = "failed",
            Some(_) => verdict = "failed",
        }
    }
    verdict
}

pub(crate) fn evidence_success_envelope(command: &str, object: Value) -> Value {
    json!({
        "schema": "planr.evidence.command.v1",
        "command": command,
        "ok": true,
        "object": object,
        "exit": {"code": evidence_object_exit_code(&object)}
    })
}

pub(crate) fn evidence_error_envelope(command: &str, error: &anyhow::Error) -> Value {
    let message = error.to_string();
    let code = evidence_error_code(error, &message);
    json!({
        "schema": "planr.evidence.command.v1",
        "command": command,
        "ok": false,
        "error": {"code": code, "message": message},
        "exit": {"code": EVIDENCE_ERROR}
    })
}

pub(crate) fn evidence_error_code(error: &anyhow::Error, message: &str) -> &'static str {
    error
        .downcast_ref::<EvidenceCommandError>()
        .map(EvidenceCommandError::code)
        .unwrap_or_else(|| crate::util::infer_error_code(message))
}

pub(crate) fn evidence_migration_request(value: &Value) -> Result<(Value, bool)> {
    let object = value.as_object().ok_or_else(|| {
        EvidenceCommandError::bad_request("evidence migrate request must be a JSON object")
    })?;
    if object.contains_key("input") || object.contains_key("apply") {
        let allowed = BTreeSet::from(["input", "apply"]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(EvidenceCommandError::bad_request(format!(
                "evidence migrate request has unknown fields: {}",
                unknown.join(",")
            ))
            .into());
        }
        let input = object.get("input").cloned().ok_or_else(|| {
            EvidenceCommandError::bad_request("evidence migrate request requires input")
        })?;
        if !input.is_object() {
            return Err(EvidenceCommandError::bad_request(
                "evidence migrate request input must be a JSON object",
            )
            .into());
        }
        let apply = match object.get("apply") {
            Some(value) => value.as_bool().ok_or_else(|| {
                EvidenceCommandError::bad_request(
                    "evidence migrate request apply must be a boolean",
                )
            })?,
            None => false,
        };
        Ok((input, apply))
    } else {
        Ok((value.clone(), false))
    }
}

pub(crate) fn evidence_envelope_exit_code(envelope: &Value) -> i32 {
    envelope["exit"]["code"].as_i64().unwrap_or(1) as i32
}

fn evidence_object_exit_code(object: &Value) -> i32 {
    match object["verdict"]
        .as_str()
        .or_else(|| object["status"].as_str())
    {
        Some("satisfied" | "waived" | "trusted" | "valid" | "passed" | "pass") | None => {
            EVIDENCE_OK
        }
        Some("unsatisfied" | "stale" | "inconclusive" | "failed" | "skipped") => {
            EVIDENCE_UNSATISFIED
        }
        Some(
            "blocked" | "unavailable" | "timed_out" | "aborted" | "permission_denied"
            | "sandbox_blocked" | "unsupported" | "probe_failed",
        ) => EVIDENCE_BLOCKED,
        Some(_) => EVIDENCE_ERROR,
    }
}

fn existing_obligation_matches(existing: &Value, obligation: &ProofObligation) -> bool {
    existing["plan_id"].as_str() == Some(obligation.plan_id.as_str())
        && existing["item_id"].as_str() == obligation.item_id.as_ref().map(|id| id.as_str())
        && existing["criterion_id"].as_str() == Some(obligation.criterion_id.as_str())
        && existing["title"].as_str() == Some(obligation.title.as_str())
        && existing["binding"].as_bool() == Some(obligation.binding)
        && existing["observations"] == serde_json::to_value(&obligation.observations).unwrap()
        && existing["fixture_policy"] == obligation.fixture_policy
        && existing["freshness_policy"] == obligation.freshness_policy
        && existing["assurance_policy"] == obligation.assurance_policy
        && existing["retry_aggregation"].as_str()
            == obligation_retry_aggregation(obligation).ok().as_deref()
        && existing["supersedes_obligation_id"].as_str()
            == obligation.supersedes.as_ref().map(|id| id.as_str())
}

fn obligation_retry_aggregation(obligation: &ProofObligation) -> Result<String> {
    match obligation
        .assurance_policy
        .get("retry_aggregation")
        .and_then(Value::as_str)
        .unwrap_or("latest_applicable_pass")
    {
        "latest_applicable_pass" => Ok("latest_applicable_pass".to_string()),
        "all_applicable_pass" => Ok("all_applicable_pass".to_string()),
        other => Err(EvidenceCommandError::bad_request(format!(
            "unsupported proof obligation retry aggregation: {other}"
        ))
        .into()),
    }
}

pub(crate) fn evidence_classifications_value() -> Value {
    json!({
        "verdict": "valid",
        "schema_version": "evidence.contract.v1",
        "canonical_gap_reasons": GapReason::ALL.iter().map(|gap| gap.as_str()).collect::<Vec<_>>(),
        "legacy_aliases": GapReason::LEGACY_ALIASES.iter().map(|(alias, canonical)| {
            json!({"alias": alias, "canonical": canonical.as_str()})
        }).collect::<Vec<_>>(),
        "host_adapters": crate::evidence::adapters::codex::host_adapter_classifications_value(),
        "unknown_legacy_reason": {
            "canonical": GapReason::canonicalize("unknown_legacy_reason").as_str(),
            "note": "unknown legacy verifier or harness reasons are verifier_failed, never product_failed"
        }
    })
}

struct ValidatedHostCapture {
    root: PathBuf,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    summary: Value,
    normalized_root_digest: String,
    validator_path: PathBuf,
    validator_digest: String,
    _tempdir: HostCaptureTempDir,
}

struct ObservedHostCaptureRun {
    import_root: PathBuf,
    provenance_source: ProvenanceSourceKind,
    producer_instance: VerificationCapabilityInstance,
    supported_observation_types: Vec<String>,
    supported_artifacts: Vec<String>,
    fixture_disclosure: FixtureDisclosure,
    fixture_disclosure_digest: String,
    adapter_binding: Value,
    resolved_command: Value,
    argv: Vec<String>,
    stdout_digest: String,
    stderr_digest: String,
    exit_code: i32,
    stdout_excerpt: String,
    stderr_excerpt: String,
    workflow_timeout_ms: u64,
    _tempdir: HostCaptureTempDir,
}

struct HostCaptureTempDir {
    path: PathBuf,
}

impl HostCaptureTempDir {
    fn create() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "planr-host-capture-import-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path)
            .with_context(|| format!("creating host capture temp dir {}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HostCaptureTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn validate_external_host_capture(
    repository_root: &Path,
    import_root: &Path,
    timeout_ms: u64,
) -> Result<ValidatedHostCapture> {
    let script = host_capability_harness_path()?;
    validate_external_host_capture_with_script(repository_root, import_root, timeout_ms, &script)
}

fn validate_external_host_capture_with_script(
    repository_root: &Path,
    import_root: &Path,
    timeout_ms: u64,
    script: &Path,
) -> Result<ValidatedHostCapture> {
    if !import_root.exists() || !import_root.is_dir() {
        return Err(EvidenceCommandError::bad_request(format!(
            "host capture import_root must be an existing directory: {}",
            import_root.display()
        ))
        .into());
    }
    let tempdir = HostCaptureTempDir::create()?;
    let out_dir = tempdir.path().join("normalized");
    if timeout_ms == 0 {
        return Err(EvidenceCommandError::bad_request(
            "host capture workflow exhausted its execution-contract timeout before validation",
        )
        .into());
    }
    let argv = vec![
        resolve_path_executable("node")?
            .to_string_lossy()
            .to_string(),
        script.to_string_lossy().to_string(),
        "capture".to_string(),
        "--out-dir".to_string(),
        out_dir.to_string_lossy().to_string(),
        "--import-fixture-root".to_string(),
        import_root.to_string_lossy().to_string(),
    ];
    let cancellation = CancellationToken::new();
    let output = run_bounded_process(BoundedProcessInput {
        cwd: repository_root,
        argv: &argv,
        env: Vec::new(),
        timeout: Duration::from_millis(timeout_ms),
        output_limit_bytes: 1_048_576,
        stdout_limit_bytes: Some(1_048_576),
        stderr_limit_bytes: Some(1_048_576),
        cancellation: &cancellation,
    })
    .with_context(|| {
        format!(
            "running bounded host capability validator {}",
            script.to_string_lossy()
        )
    })?;
    if output.timed_out {
        return Err(EvidenceCommandError::bad_request(
            "host capture validator exceeded its execution-contract timeout",
        )
        .into());
    }
    if output.interrupted {
        return Err(
            EvidenceCommandError::bad_request("host capture validator was interrupted").into(),
        );
    }
    if output.exit_code != Some(0) {
        return Err(EvidenceCommandError::bad_request(format!(
            "host capture validation failed: {}",
            output.stderr_excerpt
        ))
        .into());
    }
    let stdout = output.stdout_excerpt.into_bytes();
    let stderr = output.stderr_excerpt.into_bytes();
    let summary: Value =
        serde_json::from_slice(&stdout).context("host capture validator stdout JSON")?;
    if summary["verdict"] != "pass" {
        return Err(
            EvidenceCommandError::bad_request("host capture validator did not pass").into(),
        );
    }
    let normalized_root_digest = digest_directory(&out_dir)?;
    let validator_digest =
        crate::canonical_json::sha256_prefixed_bytes(&fs::read(script).with_context(|| {
            format!(
                "reading host capability validator {}",
                script.to_string_lossy()
            )
        })?);
    Ok(ValidatedHostCapture {
        root: out_dir,
        stdout,
        stderr,
        summary,
        normalized_root_digest,
        validator_path: script.to_path_buf(),
        validator_digest,
        _tempdir: tempdir,
    })
}

fn resolve_path_executable(program: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| anyhow!("PATH is unavailable while resolving {program}"))?;
    let candidates = std::env::split_paths(&path).flat_map(|directory| {
        #[cfg(windows)]
        let names = [program.to_string(), format!("{program}.exe")];
        #[cfg(not(windows))]
        let names = [program.to_string()];
        names.into_iter().map(move |name| directory.join(name))
    });
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| anyhow!("{program} executable is not available on PATH"))
}

fn run_planr_observed_host_capture(
    app: &App,
    value: &Value,
    obligation_id: &str,
    before_run: impl FnOnce(u64) -> Result<bool>,
) -> Result<ObservedHostCaptureRun> {
    let manifest_id = string_field(value, "manifest_id")?;
    let obligation_suffix = short_digest(&crate::canonical_json::sha256_prefixed_bytes(
        obligation_id.as_bytes(),
    ));
    let document = app.evidence_policy_document()?.ok_or_else(|| {
        EvidenceCommandError::bad_request(".planr/evidence.yaml is required for host capture run")
    })?;
    let registry = app.evidence_registry_from_policy(&document)?;
    if !registry.diagnostics().is_empty() {
        return Err(EvidenceCommandError::bad_request(
            "host capture run requires a valid Evidence capability registry",
        )
        .into());
    }
    let capability = registry
        .capabilities()
        .find(|capability| capability.manifest.id.as_str() == manifest_id)
        .ok_or_else(|| {
            EvidenceCommandError::bad_request(format!(
                "host capture run manifest is not registered: {manifest_id}"
            ))
        })?;
    if capability.manifest.provenance_path != ProvenanceSourceKind::PlanrObservedExecution {
        return Err(EvidenceCommandError::bad_request(
            "host capture run manifest must declare planr_observed_execution provenance",
        )
        .into());
    }
    if !capability
        .manifest
        .supported_surfaces
        .iter()
        .any(|surface| surface == "host-capture-run")
    {
        return Err(EvidenceCommandError::bad_request(
            "host capture run manifest must support host-capture-run surface",
        )
        .into());
    }
    let execution = capability
        .repository_execution_contract
        .as_ref()
        .unwrap_or(&capability.manifest.availability_probe.execution);
    if !before_run(execution.timeout_ms)? {
        bail!("host_capture_budget_hold");
    }

    let tempdir = HostCaptureTempDir::create()?;
    let import_root = tempdir.path().join("capture");
    let env = BTreeMap::from([(
        "PLANR_HOST_CAPTURE_OUT_DIR".to_string(),
        import_root.to_string_lossy().to_string(),
    )]);
    let resolved = resolve_process_run(&app.root, execution, &env).map_err(|error| {
        EvidenceCommandError::bad_request(format!(
            "host capture run helper resolution failed: {error}"
        ))
    })?;
    let adapter_binding = ensure_host_capture_run_adapter_digest(
        &resolved,
        execution,
        &capability.manifest.adapter_digest,
    )?;
    let cancellation = CancellationToken::new();
    let output = run_resolved_process(&resolved, execution, &cancellation)
        .with_context(|| format!("running host capture manifest {manifest_id}"))?;
    if output.timed_out {
        return Err(EvidenceCommandError::bad_request("host capture run helper timed out").into());
    }
    if output.interrupted {
        return Err(
            EvidenceCommandError::bad_request("host capture run helper was interrupted").into(),
        );
    }
    let exit_code = output.exit_code.unwrap_or(-1);
    if exit_code != 0 {
        return Err(EvidenceCommandError::bad_request(format!(
            "host capture run helper failed with exit code {exit_code}: {}",
            output.stderr_excerpt
        ))
        .into());
    }
    if !import_root.join("external-capture-envelope.json").is_file() {
        return Err(EvidenceCommandError::bad_request(
            "host capture run helper did not produce external-capture-envelope.json",
        )
        .into());
    }
    let (fixture_disclosure, fixture_disclosure_digest) = read_host_capture_run_fixture_disclosure(
        &import_root,
        &manifest_id,
        capability
            .manifest
            .supported_artifacts
            .iter()
            .any(|artifact| artifact == "host-capture-fixture-copy"),
    )?;
    let producer_instance_value = json!({
        "id": format!("host-capture-run-{}-{}", manifest_id, obligation_suffix),
        "schema_version": "evidence.contract.v1",
        "manifest_id": capability.manifest.id.as_str(),
        "manifest_digest": capability.manifest_digest,
        "host": "planr",
        "surface": "host-capture-run",
        "host_version": env!("CARGO_PKG_VERSION"),
        "adapter_version": capability.manifest.version,
        "environment": {
            "kind": "planr-host-capture-run",
            "id": format!("env-{}", manifest_id),
            "digest": crate::canonical_json::sha256_json_digest(&json!({
                "kind": "planr-host-capture-run",
                "id": format!("env-{}", manifest_id),
            }))?,
        },
        "permissions": capability.manifest.permissions,
        "availability": {
            "status": "available",
            "reason": "policy-registered host capture helper executed successfully under Planr",
        },
        "probe_result": {
            "probe_execution_id": format!("probe-{}-{}", manifest_id, obligation_suffix),
            "outcome": "passed",
            "observed_at": timestamp()?,
            "checks": [{
                "name": "policy-registered-host-capture-run",
                "outcome": "passed",
                "detail": "registered helper exited 0 and produced a strict host capture bundle",
            }],
        },
        "observed_payload_contract": {
            "schema_ref": execution.payload_schema.schema_ref,
            "observation_types": capability.manifest.supported_observations.iter().map(|schema| schema.observation_type.as_str()).collect::<Vec<_>>(),
        },
        "limitations": capability.manifest.blind_spots,
        "captured_at": timestamp()?,
    });
    let producer_instance: VerificationCapabilityInstance =
        serde_json::from_value(producer_instance_value.clone())
            .context("building host capture run producer capability instance")?;

    Ok(ObservedHostCaptureRun {
        import_root,
        provenance_source: capability.manifest.provenance_path,
        producer_instance,
        supported_observation_types: capability
            .manifest
            .supported_observations
            .iter()
            .map(|schema| schema.observation_type.as_str().to_string())
            .collect(),
        supported_artifacts: capability.manifest.supported_artifacts.clone(),
        fixture_disclosure,
        fixture_disclosure_digest,
        adapter_binding,
        resolved_command: resolved.command_identity,
        argv: output.argv,
        stdout_digest: output.stdout_digest,
        stderr_digest: output.stderr_digest,
        exit_code,
        stdout_excerpt: output.stdout_excerpt,
        stderr_excerpt: output.stderr_excerpt,
        workflow_timeout_ms: execution.timeout_ms,
        _tempdir: tempdir,
    })
}

fn ensure_host_capture_run_manifest_supports_observation(
    run: &ObservedHostCaptureRun,
    observation: &crate::evidence::model::ObservationRequirement,
) -> Result<()> {
    let observation_type = observation.observation_type.as_str();
    if !run
        .supported_observation_types
        .iter()
        .any(|supported| supported == observation_type)
    {
        return Err(EvidenceCommandError::bad_request(format!(
            "host capture run manifest {} does not support observation type {}",
            run.producer_instance.manifest_id.as_str(),
            observation_type
        ))
        .into());
    }
    if !run
        .supported_artifacts
        .iter()
        .any(|artifact| artifact == "cdp-json-result")
    {
        return Err(EvidenceCommandError::bad_request(format!(
            "host capture run manifest {} does not support cdp-json-result artifacts",
            run.producer_instance.manifest_id.as_str()
        ))
        .into());
    }
    Ok(())
}

fn ensure_host_capture_run_adapter_digest(
    resolved: &crate::evidence::execution::ResolvedProcessRun,
    execution: &ProcessExecutionContract,
    registered_digest: &Sha256Digest,
) -> Result<Value> {
    if execution.args.is_empty() {
        return Err(EvidenceCommandError::bad_request(
            "host capture run execution must name a helper file as its first argument",
        )
        .into());
    }
    let helper = resolved.file_argument_identity(0).map_err(|error| {
        EvidenceCommandError::bad_request(format!(
            "host capture run helper identity failed: {error}"
        ))
    })?;
    let binding = json!({
        "schema_version": "planr.host_capture_run.adapter_binding.v1",
        "execution_contract": execution,
        "helper": helper,
    });
    let actual_digest = crate::canonical_json::sha256_json_digest(&binding)?;
    if actual_digest != registered_digest.as_str() {
        return Err(EvidenceCommandError::bad_request(format!(
            "host capture run adapter_digest drift: manifest declares {}, actual helper/config digest is {}",
            registered_digest.as_str(),
            actual_digest
        ))
        .into());
    }
    Ok(json!({
        "digest": actual_digest,
        "binding": binding,
    }))
}

fn read_host_capture_run_fixture_disclosure(
    import_root: &Path,
    manifest_id: &str,
    requires_fixture_disclosure: bool,
) -> Result<(FixtureDisclosure, String)> {
    let disclosure_path = import_root.join("producer-disclosure.json");
    if !disclosure_path.is_file() {
        return Err(EvidenceCommandError::bad_request(
            "host capture run helper did not produce producer-disclosure.json",
        )
        .into());
    }
    let disclosure_bytes = fs::read(&disclosure_path)
        .with_context(|| format!("reading {}", disclosure_path.display()))?;
    let disclosure_digest = crate::canonical_json::sha256_prefixed_bytes(&disclosure_bytes);
    let value: Value =
        serde_json::from_slice(&disclosure_bytes).context("parsing producer-disclosure.json")?;
    let object = value.as_object().ok_or_else(|| {
        EvidenceCommandError::bad_request("producer-disclosure.json must be a JSON object")
    })?;
    let allowed = BTreeSet::from([
        "schema_version",
        "producer_manifest_id",
        "fixtures_used",
        "mocks_used",
        "fixture_refs",
        "mock_refs",
    ]);
    let unknown = object
        .keys()
        .filter(|key| !allowed.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(EvidenceCommandError::bad_request(format!(
            "producer-disclosure.json has unknown fields: {}",
            unknown.join(",")
        ))
        .into());
    }
    if string_field(&value, "schema_version")?
        != "planr.evidence.host_capture.producer_disclosure.v1"
    {
        return Err(EvidenceCommandError::bad_request(
            "unsupported producer-disclosure.json schema_version",
        )
        .into());
    }
    if string_field(&value, "producer_manifest_id")? != manifest_id {
        return Err(EvidenceCommandError::bad_request(
            "producer-disclosure.json manifest id does not match executed helper",
        )
        .into());
    }
    let fixtures_used = value
        .get("fixtures_used")
        .and_then(Value::as_bool)
        .ok_or_else(|| EvidenceCommandError::bad_request("fixtures_used must be a boolean"))?;
    let mocks_used = value
        .get("mocks_used")
        .and_then(Value::as_bool)
        .ok_or_else(|| EvidenceCommandError::bad_request("mocks_used must be a boolean"))?;
    let fixture_refs = disclosure_ref_strings(import_root, &value, "fixture_refs")?;
    let mock_refs = disclosure_ref_strings(import_root, &value, "mock_refs")?;
    if requires_fixture_disclosure && !fixtures_used {
        return Err(EvidenceCommandError::bad_request(
            "host capture run manifest declares fixture-copy production but disclosed fixtures_used=false",
        )
        .into());
    }
    if fixtures_used && fixture_refs.as_ref().is_none_or(Vec::is_empty) {
        return Err(EvidenceCommandError::bad_request(
            "producer fixture disclosure must name fixture refs when fixtures are used",
        )
        .into());
    }
    if mocks_used && mock_refs.as_ref().is_none_or(Vec::is_empty) {
        return Err(EvidenceCommandError::bad_request(
            "producer fixture disclosure must name mock refs when mocks are used",
        )
        .into());
    }
    Ok((
        FixtureDisclosure {
            fixtures_used,
            mocks_used,
            fixture_refs,
            mock_refs,
        },
        disclosure_digest,
    ))
}

fn disclosure_ref_strings(
    import_root: &Path,
    value: &Value,
    field: &str,
) -> Result<Option<Vec<String>>> {
    let Some(refs) = value.get(field) else {
        return Ok(None);
    };
    let refs = refs.as_array().ok_or_else(|| {
        EvidenceCommandError::bad_request(format!("producer disclosure {field} must be an array"))
    })?;
    let mut normalized = Vec::with_capacity(refs.len());
    for reference in refs {
        let object = reference.as_object().ok_or_else(|| {
            EvidenceCommandError::bad_request(format!(
                "producer disclosure {field} entries must be objects"
            ))
        })?;
        let allowed = BTreeSet::from(["kind", "path", "digest"]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(EvidenceCommandError::bad_request(format!(
                "producer disclosure {field} entry has unknown fields: {}",
                unknown.join(",")
            ))
            .into());
        }
        let kind = string_field(reference, "kind")?;
        let path = string_field(reference, "path")?;
        let expected_digest = string_field(reference, "digest")?;
        let actual_digest = digest_contained_file(import_root, &path)?;
        if actual_digest != expected_digest {
            return Err(EvidenceCommandError::bad_request(format!(
                "producer disclosure digest mismatch for {path}: expected {expected_digest}, actual {actual_digest}"
            ))
            .into());
        }
        normalized.push(format!("{kind}:{path}@{actual_digest}"));
    }
    Ok(Some(normalized))
}

fn digest_contained_file(root: &Path, relative: &str) -> Result<String> {
    digest_file_under_root(root, relative, "producer disclosure path")
}

fn digest_file_under_root(root: &Path, relative: &str, label: &str) -> Result<String> {
    validate_relative_file_path(relative, label)?;
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let candidate = canonical_root.join(relative);
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", candidate.display()))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(EvidenceCommandError::bad_request(format!("{label} escapes root")).into());
    }
    let bytes = fs::read(&canonical).with_context(|| format!("reading {}", canonical.display()))?;
    Ok(crate::canonical_json::sha256_prefixed_bytes(&bytes))
}

fn validate_relative_file_path(relative: &str, label: &str) -> Result<()> {
    if relative.is_empty() {
        return Err(EvidenceCommandError::bad_request(format!("{label} must be non-empty")).into());
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(EvidenceCommandError::bad_request(format!("{label} must be relative")).into());
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(EvidenceCommandError::bad_request(format!(
                    "{label} must stay within root"
                ))
                .into());
            }
        }
    }
    Ok(())
}

fn host_capability_harness_path() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("PLANR_HOST_CAPABILITY_HARNESS") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        candidates.push(bin_dir.join("scripts/host-capability-experiment.mjs"));
        candidates.push(bin_dir.join("../scripts/host-capability-experiment.mjs"));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/host-capability-experiment.mjs"),
    );
    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| anyhow!("host capability harness script is not installed"))
}

fn resolve_evidence_input_path(repository_root: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    }
}

fn digest_directory(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_directory_files(root, root, &mut files)?;
    files.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["path"].as_str().unwrap_or_default())
    });
    crate::canonical_json::sha256_json_digest(&json!({
        "schema_version": "planr.host_capture.normalized_root_digest.v1",
        "files": files,
    }))
}

fn collect_directory_files(root: &Path, dir: &Path, files: &mut Vec<Value>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_directory_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .context("normalized capture path escaped root")?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(json!({
                "path": relative,
                "digest": crate::canonical_json::sha256_prefixed_bytes(&fs::read(&path)?),
            }));
        }
    }
    Ok(())
}

fn ensure_host_import_bindings(
    obligation: &ProofObligation,
    instance: &VerificationCapabilityInstance,
    target: &TargetBinding,
    environment: &EnvironmentBinding,
    fixture_disclosure: &FixtureDisclosure,
) -> Result<()> {
    if serde_json::to_value(environment)? != serde_json::to_value(&instance.environment)? {
        return Err(EvidenceCommandError::bad_request(
            "host capture environment does not match imported capability instance",
        )
        .into());
    }
    let target_value = serde_json::to_value(target)?;
    for observation in &obligation.observations {
        if observation.target != target_value {
            return Err(EvidenceCommandError::bad_request(
                "proof obligation target does not match host capture import target",
            )
            .into());
        }
        if !instance
            .observed_payload_contract
            .observation_types
            .iter()
            .any(|observed| observed == &observation.observation_type)
        {
            return Err(EvidenceCommandError::bad_request(
                "proof obligation observation type is not supported by host capture instance",
            )
            .into());
        }
    }
    ensure_host_fixture_disclosure_allowed(&obligation.fixture_policy, fixture_disclosure)
}

fn ensure_host_fixture_disclosure_allowed(
    fixture_policy: &Value,
    disclosure: &FixtureDisclosure,
) -> Result<()> {
    let fixtures_allowed = fixture_policy
        .get("fixtures_allowed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mocks_allowed = fixture_policy
        .get("mocks_allowed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let disclosure_required = fixture_policy
        .get("disclosure_required")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if disclosure.fixtures_used && !fixtures_allowed {
        return Err(EvidenceCommandError::bad_request(
            "fixture disclosure uses fixtures disallowed by proof obligation policy",
        )
        .into());
    }
    if disclosure.mocks_used && !mocks_allowed {
        return Err(EvidenceCommandError::bad_request(
            "fixture disclosure uses mocks disallowed by proof obligation policy",
        )
        .into());
    }
    if disclosure_required {
        if disclosure.fixtures_used
            && disclosure
                .fixture_refs
                .as_ref()
                .is_none_or(|refs| refs.is_empty())
        {
            return Err(EvidenceCommandError::bad_request(
                "fixture disclosure must name fixture refs when fixtures are used",
            )
            .into());
        }
        if disclosure.mocks_used
            && disclosure
                .mock_refs
                .as_ref()
                .is_none_or(|refs| refs.is_empty())
        {
            return Err(EvidenceCommandError::bad_request(
                "fixture disclosure must name mock refs when mocks are used",
            )
            .into());
        }
    }
    Ok(())
}

fn ensure_host_capture_fresh(instance: &VerificationCapabilityInstance) -> Result<String> {
    let captured_at = OffsetDateTime::parse(&instance.captured_at, &Rfc3339)
        .context("host capture instance captured_at is not RFC3339")?;
    let now = OffsetDateTime::now_utc();
    if captured_at > now + time::Duration::minutes(5) {
        return Err(EvidenceCommandError::bad_request(
            "host capture captured_at must not be in the future",
        )
        .into());
    }
    if now - captured_at > time::Duration::minutes(5) {
        return Err(EvidenceCommandError::bad_request(
            "host capture is stale for live Evidence import",
        )
        .into());
    }
    let valid_until = captured_at + time::Duration::minutes(15);
    if valid_until <= now {
        return Err(EvidenceCommandError::bad_request(
            "host capture capability instance valid_until is expired",
        )
        .into());
    }
    Ok(valid_until.format(&Rfc3339)?)
}

fn ensure_host_capture_target_matches(
    target: &TargetBinding,
    final_event_payload: &Value,
) -> Result<()> {
    let Some(observed_url) = final_event_payload.get("url").and_then(Value::as_str) else {
        return Err(EvidenceCommandError::bad_request(
            "host capture final event does not include an observed url",
        )
        .into());
    };
    if target.uri.as_deref() != Some(observed_url) {
        return Err(EvidenceCommandError::bad_request(format!(
            "proof obligation target uri does not match host capture observed url {observed_url}"
        ))
        .into());
    }
    Ok(())
}

fn ensure_expected_predicate_matches_capture(expected: &Value, actual: &Value) -> Result<()> {
    let expected = expected.as_object().ok_or_else(|| {
        EvidenceCommandError::bad_request("host capture expected must be an object")
    })?;
    if expected.is_empty() {
        return Err(EvidenceCommandError::bad_request(
            "host capture expected must include at least one predicate field",
        )
        .into());
    }
    for (key, expected_value) in expected {
        if actual.get(key) != Some(expected_value) {
            return Err(EvidenceCommandError::bad_request(format!(
                "host capture expected.{key} does not match observed final event"
            ))
            .into());
        }
    }
    Ok(())
}

fn host_capture_attempt_id(
    obligation_id: &EvidenceId,
    capability_instance_id: &EvidenceId,
    raw_digest: &str,
    execution_binding: &Value,
) -> Result<String> {
    let now = timestamp()?;
    let nonce = uuid::Uuid::new_v4();
    let execution_binding_digest = crate::canonical_json::sha256_json_digest(execution_binding)?;
    let digest = crate::canonical_json::sha256_prefixed_bytes(
        format!(
            "{}:{}:{}:{execution_binding_digest}:{raw_digest}:{nonce}",
            obligation_id.as_str(),
            capability_instance_id.as_str(),
            now
        )
        .as_bytes(),
    );
    Ok(format!("eatt-{}", short_digest(&digest)))
}

fn short_digest(value: &str) -> String {
    value
        .trim_start_matches("sha256:")
        .chars()
        .take(12)
        .collect()
}

fn map_from_json_object(value: Value, label: &str) -> Result<serde_json::Map<String, Value>> {
    value
        .as_object()
        .cloned()
        .with_context(|| format!("{label} must be a JSON object"))
}

fn read_json_file(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn timestamp() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn string_field(value: &Value, field: &'static str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("missing required Evidence field: {field}"))
}

fn string_ref_field<'a>(value: &'a Value, field: &'static str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("missing required Evidence field: {field}"))
}

const PUBLIC_EVIDENCE_RUN_REJECTED_TRUST_FIELDS: &[&str] = &[
    "attempt",
    "receipt",
    "receipt_json",
    "trusted_binding_json",
    "trusted_receipt",
    "receipt_status",
    "provenance",
];

fn reject_trusted_receipt_input(value: &Value) -> Result<()> {
    for field in PUBLIC_EVIDENCE_RUN_REJECTED_TRUST_FIELDS {
        if value.get(field).is_some() {
            return Err(EvidenceCommandError::bad_request(format!(
                "public Evidence input cannot construct trusted receipt field: {field}"
            ))
            .into());
        }
    }
    Ok(())
}

fn registry_diagnostics_value(
    diagnostics: Vec<crate::evidence::registry::CapabilityRegistryDiagnostic>,
) -> Vec<Value> {
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            json!({
                "manifest_id": diagnostic.manifest_id,
                "code": format!("{:?}", diagnostic.code),
                "message": diagnostic.message,
            })
        })
        .collect()
}

fn empty_registry_probe(diagnostics: Value) -> Value {
    json!({
        "registered_capabilities": 0,
        "available_capabilities": 0,
        "probes": [],
        "diagnostics": diagnostics,
    })
}

fn policy_text_has_no_adapters(root: &Path) -> bool {
    let path = root.join(".planr/evidence.yaml");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_yaml::from_str::<Value>(&text) else {
        return false;
    };
    value
        .get("adapter_registrations")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

fn evidence_doctor_policy_state(policy: &Value) -> &'static str {
    if policy["status"] == "absent" {
        return "missing_policy";
    }
    if policy["status"] == "invalid" {
        return "malformed_policy";
    }
    let registered = policy["registry"]["registered_capabilities"]
        .as_u64()
        .unwrap_or(0);
    if registered == 0 {
        return "no_adapters";
    }
    let diagnostics = policy["diagnostics"].as_array().map_or(0, Vec::len)
        + policy["registry"]["diagnostics"]
            .as_array()
            .map_or(0, Vec::len);
    let probes = policy["registry"]["probes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if probes.iter().any(|probe| {
        matches!(
            probe["availability_status"].as_str(),
            Some(
                "degraded"
                    | "permission_denied"
                    | "sandbox_blocked"
                    | "unsupported"
                    | "probe_failed"
            )
        )
    }) {
        return "degraded";
    }
    if probes
        .iter()
        .any(|probe| probe["availability_status"].as_str() != Some("available"))
    {
        return "unavailable";
    }
    if probes.iter().any(|probe| {
        matches!(
            probe["resolution"].as_str(),
            Some(
                "reprobed_expired"
                    | "reprobed_runtime_mismatch"
                    | "reprobed_environment_mismatch"
                    | "reprobed_registration_mismatch"
            )
        )
    }) {
        return "recovered";
    }
    if diagnostics == 0 && policy["status"] == "valid" {
        "ready"
    } else {
        "degraded"
    }
}

fn proposal_value(proposal: &crate::evidence::UntrustedEvidenceProposal) -> Value {
    json!({
        "id": proposal.id,
        "schema_version": proposal.schema_version,
        "source_kind": proposal.source_kind,
        "submitted_at": proposal.submitted_at,
        "claims": proposal.claims,
        "artifact_refs": proposal.artifact_refs.iter().map(|artifact| json!({
            "id": artifact.id,
            "kind": artifact.kind,
            "digest": artifact.digest,
            "uri": artifact.uri,
        })).collect::<Vec<_>>(),
        "producer_metadata": proposal.producer_metadata,
    })
}

fn parse_json(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn obligation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let obligation_shape = row.get::<_, String>(18)?;
    let mut value = json!({
        "id": row.get::<_, String>(0)?,
        "project_id": row.get::<_, String>(1)?,
        "plan_id": row.get::<_, String>(2)?,
        "item_id": row.get::<_, Option<String>>(3)?,
        "criterion_id": row.get::<_, String>(4)?,
        "title": row.get::<_, String>(5)?,
        "binding": row.get::<_, i64>(6)? == 1,
        "observations": parse_json(row.get::<_, String>(7)?),
        "fixture_policy": parse_json(row.get::<_, String>(8)?),
        "freshness_policy": parse_json(row.get::<_, String>(9)?),
        "assurance_policy": parse_json(row.get::<_, String>(10)?),
        "retry_aggregation": row.get::<_, String>(11)?,
        "policy_digest": row.get::<_, String>(12)?,
        "config_digest": row.get::<_, String>(13)?,
        "source_digest": row.get::<_, Option<String>>(14)?,
        "supersedes_obligation_id": row.get::<_, Option<String>>(15)?,
        "obligation_version": row.get::<_, i64>(16)?,
        "created_at": row.get::<_, String>(17)?,
        "obligation_shape": obligation_shape.as_str(),
    });
    if obligation_shape == "semantic_v1" {
        let object = value
            .as_object_mut()
            .expect("obligation record is constructed as an object");
        object.remove("policy_digest");
        object.remove("config_digest");
        object.remove("source_digest");
    }
    Ok(value)
}

fn manifest_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "version": row.get::<_, String>(1)?,
        "adapter_kind": row.get::<_, String>(2)?,
        "adapter_digest": row.get::<_, String>(3)?,
        "manifest_digest": row.get::<_, String>(4)?,
        "manifest": parse_json(row.get::<_, String>(5)?),
        "source_path": row.get::<_, Option<String>>(6)?,
        "created_at": row.get::<_, String>(7)?,
    }))
}

fn capability_instance_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "manifest_id": row.get::<_, String>(1)?,
        "manifest_version": row.get::<_, String>(2)?,
        "manifest_digest": row.get::<_, String>(3)?,
        "probe_execution_id": row.get::<_, String>(4)?,
        "availability_status": row.get::<_, String>(5)?,
        "runtime_target": parse_json(row.get::<_, String>(6)?),
        "host_fingerprint": parse_json(row.get::<_, String>(7)?),
        "capability": parse_json(row.get::<_, String>(8)?),
        "probe_result": parse_json(row.get::<_, String>(9)?),
        "created_at": row.get::<_, String>(10)?,
        "valid_until": row.get::<_, Option<String>>(11)?,
    }))
}

fn attempt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "project_id": row.get::<_, String>(1)?,
        "obligation_id": row.get::<_, String>(2)?,
        "capability_instance_id": row.get::<_, String>(3)?,
        "status": row.get::<_, String>(4)?,
        "execution_contract_digest": row.get::<_, String>(5)?,
        "resolved_command": parse_json(row.get::<_, String>(6)?),
        "environment_digest": row.get::<_, String>(7)?,
        "retry_predecessor_attempt_id": row.get::<_, Option<String>>(8)?,
        "started_at": row.get::<_, String>(9)?,
        "completed_at": row.get::<_, Option<String>>(10)?,
        "exit_code": row.get::<_, Option<i64>>(11)?,
        "stdout_digest": row.get::<_, Option<String>>(12)?,
        "stderr_digest": row.get::<_, Option<String>>(13)?,
        "output_bounds": parse_json(row.get::<_, String>(14)?),
        "attempt": parse_json(row.get::<_, String>(15)?),
        "created_at": row.get::<_, String>(16)?,
    }))
}

fn receipt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let receipt = parse_json(row.get::<_, String>(9)?);
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "project_id": row.get::<_, String>(1)?,
        "obligation_id": row.get::<_, String>(2)?,
        "attempt_id": row.get::<_, String>(3)?,
        "status": row.get::<_, String>(4)?,
        "receipt_digest": row.get::<_, String>(5)?,
        "trusted_binding": parse_json(row.get::<_, String>(6)?),
        "observations": parse_json(row.get::<_, String>(7)?),
        "provenance": parse_json(row.get::<_, String>(8)?),
        "receipt": receipt,
        "supersedes_receipt_id": row.get::<_, Option<String>>(10)?,
        "created_at": row.get::<_, String>(11)?,
    }))
}

fn query_json_rows<P>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: P,
    mapper: fn(&rusqlite::Row<'_>) -> rusqlite::Result<Value>,
) -> Result<Vec<Value>>
where
    P: rusqlite::Params,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, mapper)?;
    crate::util::collect_rows(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_evidence_input_rejects_agent_supplied_attempt_and_receipt_objects() {
        for field in PUBLIC_EVIDENCE_RUN_REJECTED_TRUST_FIELDS {
            let mut input = json!({});
            input[field] = json!({"id": "forged"});
            let error = reject_trusted_receipt_input(&input).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("cannot construct trusted receipt field"),
                "{field}: {error}"
            );
        }
    }

    #[test]
    fn evidence_exit_codes_treat_waived_failed_and_blocked_statuses_distinctly() {
        assert_eq!(
            evidence_success_envelope("evidence.coverage", json!({"verdict": "waived"}))["exit"]["code"],
            EVIDENCE_OK
        );
        assert_eq!(
            evidence_success_envelope("evidence.run", json!({"verdict": "failed"}))["exit"]["code"],
            EVIDENCE_UNSATISFIED
        );
        assert_eq!(
            evidence_success_envelope("evidence.run", json!({"verdict": "blocked"}))["exit"]["code"],
            EVIDENCE_BLOCKED
        );
        assert_eq!(
            evidence_run_index_verdict(&[json!({"verdict": "passed"})]),
            "passed"
        );
        for (outcome, expected, exit) in [
            ("failed", "failed", EVIDENCE_UNSATISFIED),
            ("inconclusive", "inconclusive", EVIDENCE_UNSATISFIED),
            ("timed_out", "timed_out", EVIDENCE_BLOCKED),
            ("aborted", "aborted", EVIDENCE_BLOCKED),
            ("unavailable", "unavailable", EVIDENCE_BLOCKED),
        ] {
            let verdict = evidence_run_index_verdict(&[
                json!({"verdict": "passed"}),
                json!({"verdict": outcome}),
            ]);
            assert_eq!(verdict, expected);
            assert_eq!(
                evidence_success_envelope("evidence.run", json!({"verdict": verdict}))["exit"]["code"],
                exit
            );
        }
    }

    #[test]
    fn evidence_run_verdict_projects_both_verifier_failure_kinds() {
        let exit = json!({"error": "verifier_failed"});
        assert_eq!(
            evidence_run_verdict(
                AttemptStatus::Failed,
                &exit,
                &json!({"ordinary_observation_error": "invalid JSON"}),
            ),
            "verifier_failed"
        );
        assert_eq!(
            evidence_run_verdict(
                AttemptStatus::Failed,
                &exit,
                &json!({"structured_observation_error": "schema mismatch"}),
            ),
            "verifier_failed"
        );
        assert_eq!(
            evidence_run_verdict(AttemptStatus::Failed, &exit, &json!({})),
            "failed"
        );
    }

    #[test]
    fn terminal_exhaustion_requires_one_shot_final_non_passing_attempt_with_lease() {
        assert_eq!(
            admitted_max_attempts("non_repeatable_one_shot", None).unwrap(),
            1
        );
        assert!(
            admitted_max_attempts("non_repeatable_one_shot", Some(2))
                .unwrap_err()
                .to_string()
                .contains("requires max_attempts = 1")
        );
        assert_eq!(admitted_max_attempts("repeatable", None).unwrap(), 3);
        assert!(should_settle_terminal_exhaustion(
            "failed",
            true,
            "non_repeatable_one_shot",
            0,
            1,
        ));
        for candidate in [
            ("passed", true, "non_repeatable_one_shot", 0, 1),
            ("failed", false, "non_repeatable_one_shot", 0, 1),
            ("failed", true, "repeatable", 0, 1),
            ("failed", true, "non_repeatable_one_shot", 0, 2),
        ] {
            assert!(!should_settle_terminal_exhaustion(
                candidate.0,
                candidate.1,
                candidate.2,
                candidate.3,
                candidate.4,
            ));
        }
    }

    #[test]
    fn hermetic_reuse_classification_excludes_network_browser_live_and_fixtures() {
        let mut manifest_value: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/verification-capability-manifest.json"
        ))
        .unwrap();
        manifest_value["permissions"]["network"] = json!("none");
        manifest_value["determinism"] = json!("deterministic");
        manifest_value["repeatability"] = json!("repeatable");
        manifest_value["supported_surfaces"] = json!(["local-process"]);
        manifest_value["supported_interactions"] = json!(["command"]);
        let manifest: VerificationCapabilityManifest =
            serde_json::from_value(manifest_value.clone()).unwrap();
        let target: TargetBinding =
            serde_json::from_value(json!({"kind": "process", "uri": "local://check"})).unwrap();
        let clean: FixtureDisclosure = serde_json::from_value(json!({
            "fixtures_used": false,
            "mocks_used": false
        }))
        .unwrap();
        assert!(is_hermetic_reuse_candidate(&manifest, &target, &clean, &BTreeMap::new()).unwrap());

        for (field, value) in [
            ("supported_surfaces", json!(["browser"])),
            ("supported_interactions", json!(["live-check"])),
        ] {
            let mut candidate = manifest_value.clone();
            candidate[field] = value;
            let candidate: VerificationCapabilityManifest =
                serde_json::from_value(candidate).unwrap();
            assert!(
                !is_hermetic_reuse_candidate(&candidate, &target, &clean, &BTreeMap::new())
                    .unwrap()
            );
        }
        let mut network = manifest_value;
        network["permissions"]["network"] = json!("loopback");
        let network: VerificationCapabilityManifest = serde_json::from_value(network).unwrap();
        assert!(!is_hermetic_reuse_candidate(&network, &target, &clean, &BTreeMap::new()).unwrap());
        let fixture: FixtureDisclosure = serde_json::from_value(json!({
            "fixtures_used": true,
            "mocks_used": false,
            "fixture_refs": ["planr-test-fixture:cache-disabled"]
        }))
        .unwrap();
        assert!(
            !is_hermetic_reuse_candidate(&manifest, &target, &fixture, &BTreeMap::new()).unwrap()
        );
    }

    #[test]
    fn hanging_host_validator_is_killed_at_the_reserved_deadline() {
        let root = tempfile::tempdir().unwrap();
        let import_root = root.path().join("capture");
        fs::create_dir(&import_root).unwrap();
        let script = root.path().join("hang.mjs");
        fs::write(&script, "setInterval(() => {}, 1000);\n").unwrap();
        let started = Instant::now();
        let error = match validate_external_host_capture_with_script(
            root.path(),
            &import_root,
            50,
            &script,
        ) {
            Ok(_) => panic!("hanging validator unexpectedly completed"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("execution-contract timeout"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
