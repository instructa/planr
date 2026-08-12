#![allow(dead_code)]

use super::adapter_signal::{AdapterBoundarySignal, adapter_boundary_signal_from_process_output};
use super::model::{
    AttemptStatus, CapabilityBinding, EnvironmentBinding, EvidenceAttempt, EvidenceId,
    FixtureDisclosure, GapReason, ObservationResult, ProcessExecutionContract, ProofObligation,
    RawResultRef, SandboxLimits, SandboxState, SchemaVersion, Sha256Digest, TargetBinding,
    TrustedProvenance, TrustedReceiptInput, VantagePoint, VerificationCapabilityInstance,
    build_trusted_receipt,
};
use super::policy::{
    EvidenceRepositorySnapshot, capture_repository_snapshot, trusted_receipt_binding_value,
};
use crate::canonical_json::{sha256_json_digest, sha256_prefixed_bytes};
use crate::execution::{
    BoundedProcessError, BoundedProcessInput, BoundedProcessOutput, CancellationToken,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fmt, fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub(crate) struct ConfiguredProcessRunInput<'a> {
    pub repository_root: &'a Path,
    pub project_id: &'a str,
    pub obligation: ProofObligation,
    pub capability_instance: VerificationCapabilityInstance,
    pub execution_contract: ProcessExecutionContract,
    pub payload_json_schema: Option<Value>,
    pub observation_payload_json_schemas: BTreeMap<String, Value>,
    pub target: TargetBinding,
    pub environment: EnvironmentBinding,
    pub fixture_disclosure: FixtureDisclosure,
    pub env: BTreeMap<String, String>,
    pub retry_of: Option<EvidenceId>,
    pub attempt_index: u32,
    pub max_attempts: u32,
    pub cancellation: &'a CancellationToken,
}

#[derive(Debug, Clone)]
pub(crate) struct ConfiguredProcessRunOutput {
    pub attempt: EvidenceAttempt,
    pub receipt_value: Value,
    pub receipt_digest: String,
}

#[derive(Debug)]
pub(crate) struct HostCapturePersistenceInput<'a> {
    pub project_id: &'a str,
    pub obligation_id: &'a EvidenceId,
    pub attempt: &'a EvidenceAttempt,
    pub execution_contract_digest: &'a str,
    pub environment_digest: &'a str,
    pub receipt_value: &'a Value,
    pub receipt_digest: &'a str,
    pub trusted_binding_value: &'a Value,
}

pub(crate) struct TrustedEvidencePersistenceInput<'a> {
    pub project_id: &'a str,
    pub obligation_id: &'a EvidenceId,
    pub attempt: &'a EvidenceAttempt,
    pub execution_contract_digest: &'a str,
    pub environment_digest: &'a str,
    pub retry_predecessor_attempt_id: Option<&'a str>,
    pub receipt_value: &'a Value,
    pub receipt_digest: &'a str,
    pub trusted_binding_value: &'a Value,
}

pub(crate) fn persist_host_capture_evidence(
    conn: &Connection,
    input: HostCapturePersistenceInput<'_>,
) -> Result<()> {
    persist_trusted_evidence_atomically(
        conn,
        |_| Ok(()),
        |_| Ok(()),
        TrustedEvidencePersistenceInput {
            project_id: input.project_id,
            obligation_id: input.obligation_id,
            attempt: input.attempt,
            execution_contract_digest: input.execution_contract_digest,
            environment_digest: input.environment_digest,
            retry_predecessor_attempt_id: None,
            receipt_value: input.receipt_value,
            receipt_digest: input.receipt_digest,
            trusted_binding_value: input.trusted_binding_value,
        },
    )
}

pub(crate) fn persist_trusted_evidence_atomically<F>(
    conn: &Connection,
    pre_attempt: F,
    pre_commit: impl FnOnce(&Connection) -> Result<()>,
    input: TrustedEvidencePersistenceInput<'_>,
) -> Result<()>
where
    F: FnOnce(&Connection) -> Result<()>,
{
    ensure_autocommit_storage_boundary(conn)?;
    let tx = conn.unchecked_transaction()?;
    pre_attempt(&tx)?;
    persist_attempt(
        &tx,
        PersistedAttempt {
            project_id: input.project_id,
            obligation_id: input.obligation_id,
            attempt: input.attempt,
            execution_contract_digest: input.execution_contract_digest,
            environment_digest: input.environment_digest,
            retry_predecessor_attempt_id: input.retry_predecessor_attempt_id,
        },
    )?;
    persist_receipt(
        &tx,
        PersistedReceipt {
            project_id: input.project_id,
            obligation_id: input.obligation_id,
            attempt: input.attempt,
            receipt_value: input.receipt_value,
            receipt_digest: input.receipt_digest,
            trusted_binding_value: input.trusted_binding_value,
            retry_predecessor_attempt_id: input.retry_predecessor_attempt_id,
        },
    )?;
    pre_commit(&tx)?;
    tx.commit()?;
    Ok(())
}

fn persist_attempt_atomically(conn: &Connection, input: PersistedAttempt<'_>) -> Result<()> {
    ensure_autocommit_storage_boundary(conn)?;
    let tx = conn.unchecked_transaction()?;
    persist_attempt(&tx, input)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn resolve_process_run(
    repository_root: &Path,
    execution: &ProcessExecutionContract,
    env_overrides: &BTreeMap<String, String>,
) -> Result<ResolvedProcessRun> {
    ResolvedProcessRun::resolve(repository_root, execution, env_overrides)
}

pub(crate) fn run_resolved_process(
    resolved: &ResolvedProcessRun,
    execution: &ProcessExecutionContract,
    cancellation: &CancellationToken,
) -> Result<BoundedProcessOutput> {
    crate::execution::run_bounded_process(BoundedProcessInput {
        cwd: &resolved.cwd,
        argv: &resolved.argv,
        env: resolved.env_for_process(),
        timeout: Duration::from_millis(execution.timeout_ms),
        output_limit_bytes: usize::MAX,
        stdout_limit_bytes: Some(limit_to_usize(execution.stdout_limit_bytes)),
        stderr_limit_bytes: Some(limit_to_usize(execution.stderr_limit_bytes)),
        cancellation,
    })
}

pub(crate) fn run_configured_process_adapter(
    conn: &Connection,
    input: ConfiguredProcessRunInput<'_>,
) -> Result<ConfiguredProcessRunOutput> {
    run_configured_process_adapter_guarded(conn, input, &|_| Ok(()), &|_, _, _| Ok(()))
}

pub(crate) fn run_configured_process_adapter_guarded(
    conn: &Connection,
    input: ConfiguredProcessRunInput<'_>,
    feature_run_guard: &dyn Fn(&Connection) -> Result<()>,
    trusted_evidence_settlement: &dyn Fn(&Connection, &EvidenceAttempt, &Value) -> Result<()>,
) -> Result<ConfiguredProcessRunOutput> {
    ensure_autocommit_storage_boundary(conn)?;
    feature_run_guard(conn)?;
    let capability_instance = registered_capability_instance(conn, &input.capability_instance.id)
        .with_context(|| {
        format!(
            "loading registered capability instance {}",
            input.capability_instance.id.as_str()
        )
    })?;
    let execution_contract_value = serde_json::to_value(&input.execution_contract)?;
    let execution_contract_digest = sha256_json_digest(&execution_contract_value)?;
    ensure_registered_execution_contract_digest(
        conn,
        &input.capability_instance.id,
        &execution_contract_digest,
    )?;
    ensure_input_instance_matches_registration(&input.capability_instance, &capability_instance)?;
    ensure_available_process_instance(&capability_instance)?;
    ensure_contract_compatibility(
        &input.obligation,
        &capability_instance,
        &input.execution_contract,
    )?;
    let repository_snapshot = capture_repository_snapshot(input.repository_root)
        .map_err(|err| anyhow::anyhow!("capturing repository evidence snapshot: {err}"))?;
    let source_binding = repository_snapshot.source.clone();
    let policy_binding = repository_snapshot.trusted_policy_binding()?;
    ensure_runtime_bindings(
        &input.obligation,
        &capability_instance,
        &input.target,
        &input.environment,
        &input.fixture_disclosure,
    )?;
    let adapter_env = evidence_adapter_env(
        &input.env,
        &input.target,
        &input.environment,
        &execution_contract_digest,
    )?;
    let retry_lineage = resolve_retry_lineage(
        conn,
        input.project_id,
        &input.obligation.id,
        &capability_instance.id,
        input.retry_of.as_ref(),
        input.attempt_index,
        input.max_attempts,
    )?;
    let resolved = match ResolvedProcessRun::resolve(
        input.repository_root,
        &input.execution_contract,
        &adapter_env,
    ) {
        Ok(resolved) => ResolvedProcessRunResolution::Runnable(resolved),
        Err(error) if is_missing_process_executable(&error) => {
            let reason = error.to_string();
            ResolvedProcessRunResolution::Unavailable(
                ResolvedProcessRun::unresolved(
                    input.repository_root,
                    &input.execution_contract,
                    &adapter_env,
                    &reason,
                )?,
                reason,
            )
        }
        Err(error) => return Err(error),
    };
    if let ResolvedProcessRunResolution::Runnable(resolved) = &resolved {
        let manifest = registered_capability_manifest(conn, &capability_instance.id)?;
        ensure_process_adapter_digest(&manifest, resolved)?;
    }
    let resolved_run = resolved.run();
    let config_digest = sha256_json_digest(&json!({
        "schema_version": "planr.evidence.runtime-config.v1",
        "capability_instance": capability_instance,
        "execution_contract": execution_contract_value,
        "target": input.target,
        "environment": input.environment,
        "fixture_disclosure": input.fixture_disclosure,
    }))?;
    let started_at = timestamp();
    let output = match &resolved {
        ResolvedProcessRunResolution::Runnable(resolved) => {
            Some(crate::execution::run_bounded_process(BoundedProcessInput {
                cwd: &resolved.cwd,
                argv: &resolved.argv,
                env: resolved.env_for_process(),
                timeout: Duration::from_millis(input.execution_contract.timeout_ms),
                output_limit_bytes: usize::MAX,
                stdout_limit_bytes: Some(limit_to_usize(
                    input.execution_contract.stdout_limit_bytes,
                )),
                stderr_limit_bytes: Some(limit_to_usize(
                    input.execution_contract.stderr_limit_bytes,
                )),
                cancellation: input.cancellation,
            }))
        }
        ResolvedProcessRunResolution::Unavailable(_, _) => None,
    };
    let ended_at = timestamp();
    let attempt_id = attempt_id(
        input.obligation.id.as_str(),
        capability_instance.id.as_str(),
        &started_at,
    );
    let mut process_result = match (&resolved, output) {
        (ResolvedProcessRunResolution::Runnable(resolved), Some(output)) => {
            attempt_result(output, resolved)?
        }
        (ResolvedProcessRunResolution::Unavailable(resolved, reason), None) => {
            unavailable_process_error_result(reason.clone(), resolved)?
        }
        _ => unreachable!("resolved process state and execution output diverged"),
    };
    let validated_observation_results = if process_result.status == AttemptStatus::Passed {
        if requires_structured_observation_results(&input.execution_contract) {
            match strict_structured_observation_results(
                &input.obligation,
                &input.execution_contract,
                &input.observation_payload_json_schemas,
                StructuredObservationContext {
                    target: &input.target,
                    environment: &input.environment,
                    fixture_disclosure: &input.fixture_disclosure,
                    repository_root: input.repository_root,
                },
                &process_result.raw_result,
            ) {
                Ok(results) => Some(results),
                Err(error) => {
                    mark_structured_observation_failure(&mut process_result, error.to_string())?;
                    None
                }
            }
        } else {
            match strict_ordinary_process_observation_results(
                &input.obligation,
                input.payload_json_schema.as_ref(),
                &process_result.raw_result,
            ) {
                Ok(results) => Some(results),
                Err(OrdinaryObservationError::Malformed(error)) => {
                    mark_ordinary_observation_verifier_failure(&mut process_result, error)?;
                    None
                }
                Err(OrdinaryObservationError::SemanticMismatch(error)) => {
                    mark_semantic_observation_mismatch(&mut process_result, error)?;
                    None
                }
            }
        }
    } else {
        None
    };
    let post_process_repository_snapshot_mismatch =
        if matches!(resolved, ResolvedProcessRunResolution::Runnable(_)) {
            repository_snapshot_mismatch(input.repository_root, &repository_snapshot).context(
                "checking repository evidence source and policy after configured process execution",
            )?
        } else {
            None
        };
    if let Some(mismatch) = &post_process_repository_snapshot_mismatch {
        mark_repository_snapshot_mismatch(&mut process_result, mismatch)?;
    }
    let attempt_from_process_result =
        |process_result: &AdapterProcessResult| -> Result<EvidenceAttempt> {
            Ok(EvidenceAttempt {
                id: EvidenceId::parse(attempt_id.clone())?,
                schema_version: SchemaVersion::v1(),
                criterion_id: input.obligation.criterion_id.clone(),
                obligation_id: input.obligation.id.clone(),
                capability_instance_id: capability_instance.id.clone(),
                started_at: started_at.clone(),
                ended_at: ended_at.clone(),
                status: process_result.status,
                resolved_command: resolved_run.command_identity.clone(),
                exit: process_result.exit.clone(),
                retry_lineage: retry_lineage.value.clone(),
                stdout_digest: Sha256Digest::parse(process_result.stdout_digest.clone())?,
                stderr_digest: Sha256Digest::parse(process_result.stderr_digest.clone())?,
                raw_result: process_result.raw_result.clone(),
                artifacts: process_result.artifacts.clone(),
                output_bounds: process_result.output_bounds.clone(),
            })
        };
    let attempt = attempt_from_process_result(&process_result)?;
    if let Some(mismatch) = post_process_repository_snapshot_mismatch {
        persist_attempt_atomically(
            conn,
            PersistedAttempt {
                project_id: input.project_id,
                obligation_id: &input.obligation.id,
                attempt: &attempt,
                execution_contract_digest: &execution_contract_digest,
                environment_digest: capability_instance.environment.digest.as_str(),
                retry_predecessor_attempt_id: retry_lineage
                    .retry_of
                    .as_ref()
                    .map(EvidenceId::as_str),
            },
        )?;
        bail!(
            "{}; recorded failed attempt {} without trusted receipt",
            mismatch.message,
            attempt.id.as_str()
        );
    }
    let raw_result_digest = sha256_json_digest(&process_result.raw_result)?;
    let instance_digest = sha256_json_digest(&serde_json::to_value(&capability_instance)?)?;
    let receipt = build_trusted_receipt(TrustedReceiptInput {
        id: EvidenceId::parse(receipt_id(&attempt_id))?,
        criterion_id: input.obligation.criterion_id.clone(),
        obligation_id: input.obligation.id.clone(),
        source: source_binding,
        target: input.target.clone(),
        environment: input.environment.clone(),
        vantage_point: VantagePoint {
            kind: "process_adapter".to_string(),
            identity: capability_instance.manifest_id.as_str().to_string(),
        },
        capability: CapabilityBinding {
            manifest_id: capability_instance.manifest_id.clone(),
            manifest_digest: capability_instance.manifest_digest.clone(),
            instance_id: capability_instance.id.clone(),
            instance_digest: Sha256Digest::parse(instance_digest)?,
        },
        provenance: TrustedProvenance::planr_observed_execution(attempt_id.clone())?,
        observations: observation_results(
            &input.obligation,
            process_result.status,
            &process_result.raw_result,
            validated_observation_results.as_ref(),
        )?,
        attempt_ids: vec![EvidenceId::parse(attempt_id.clone())?],
        retry_history: retry_history(&retry_lineage, process_result.status),
        artifacts: vec![],
        raw_result: RawResultRef {
            kind: "process_result".to_string(),
            digest: Sha256Digest::parse(raw_result_digest)?,
            artifact_id: None,
            extra: Map::new(),
        },
        config_digest: Sha256Digest::parse(config_digest.clone())?,
        fixture_disclosure: input.fixture_disclosure.clone(),
        permissions: capability_instance.permissions.clone(),
        sandbox: SandboxState {
            mode: "bounded_process".to_string(),
            limits: SandboxLimits {
                timeout_ms: input.execution_contract.timeout_ms,
                stdout_bytes: input.execution_contract.stdout_limit_bytes,
                stderr_bytes: input.execution_contract.stderr_limit_bytes,
            },
        },
        proof_gaps: proof_gaps(
            process_result.status,
            &process_result.exit,
            &process_result.raw_result,
        ),
        started_at: started_at.clone(),
        ended_at: ended_at.clone(),
    })?;
    let trusted_binding_value = trusted_receipt_binding_value(&receipt, policy_binding)
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    let receipt_value = serde_json::to_value(&receipt)?;
    let receipt_digest = string_field(&receipt_value, "receipt_digest")?.to_string();
    let persistence_result = persist_trusted_evidence_atomically(
        conn,
        |tx| feature_run_guard(tx),
        |tx| {
            run_repository_snapshot_pre_commit_test_hook(input.repository_root)?;
            if let Some(mismatch) =
                repository_snapshot_mismatch(input.repository_root, &repository_snapshot).context(
                    "checking repository evidence source and policy immediately before trusted receipt commit",
                )?
            {
                bail!(RepositorySnapshotPreCommitMismatch { mismatch });
            }
            feature_run_guard(tx)?;
            trusted_evidence_settlement(tx, &attempt, &receipt_value)?;
            Ok(())
        },
        TrustedEvidencePersistenceInput {
            project_id: input.project_id,
            obligation_id: &input.obligation.id,
            attempt: &attempt,
            execution_contract_digest: &execution_contract_digest,
            environment_digest: capability_instance.environment.digest.as_str(),
            retry_predecessor_attempt_id: retry_lineage.retry_of.as_ref().map(EvidenceId::as_str),
            receipt_value: &receipt_value,
            receipt_digest: &receipt_digest,
            trusted_binding_value: &trusted_binding_value,
        },
    );
    if let Err(error) = persistence_result {
        let RepositorySnapshotPreCommitMismatch { mismatch } =
            error.downcast::<RepositorySnapshotPreCommitMismatch>()?;
        mark_repository_snapshot_mismatch(&mut process_result, &mismatch)?;
        let failed_attempt = attempt_from_process_result(&process_result)?;
        persist_attempt_atomically(
            conn,
            PersistedAttempt {
                project_id: input.project_id,
                obligation_id: &input.obligation.id,
                attempt: &failed_attempt,
                execution_contract_digest: &execution_contract_digest,
                environment_digest: capability_instance.environment.digest.as_str(),
                retry_predecessor_attempt_id: retry_lineage
                    .retry_of
                    .as_ref()
                    .map(EvidenceId::as_str),
            },
        )?;
        bail!(
            "{}; recorded failed attempt {} without trusted receipt",
            mismatch.message,
            failed_attempt.id.as_str()
        );
    }
    Ok(ConfiguredProcessRunOutput {
        attempt,
        receipt_value,
        receipt_digest,
    })
}

fn ensure_autocommit_storage_boundary(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        bail!(
            "configured process adapter requires an autocommit SQLite connection for atomic evidence persistence"
        );
    }
    Ok(())
}

fn registered_capability_instance(
    conn: &Connection,
    instance_id: &EvidenceId,
) -> Result<VerificationCapabilityInstance> {
    let snapshot: String = conn.query_row(
        "SELECT capability_snapshot_json FROM verification_capability_instances WHERE id = ?1",
        [instance_id.as_str()],
        |row| row.get(0),
    )?;
    serde_json::from_str(&snapshot).context("decoding registered capability instance snapshot")
}

fn registered_capability_manifest(
    conn: &Connection,
    capability_instance_id: &EvidenceId,
) -> Result<super::model::VerificationCapabilityManifest> {
    let snapshot: String = conn.query_row(
        "SELECT manifests.manifest_json
         FROM verification_capability_instances instances
         JOIN verification_capability_manifests manifests
           ON manifests.id = instances.manifest_id
          AND manifests.version = instances.manifest_version
         WHERE instances.id = ?1",
        [capability_instance_id.as_str()],
        |row| row.get(0),
    )?;
    serde_json::from_str(&snapshot).context("decoding registered capability manifest snapshot")
}

pub(crate) fn ensure_process_adapter_digest(
    manifest: &super::model::VerificationCapabilityManifest,
    resolved: &ResolvedProcessRun,
) -> Result<()> {
    let binding = process_adapter_binding(resolved, &manifest.availability_probe.execution)?;
    let actual_digest = sha256_json_digest(&binding)?;
    if actual_digest != manifest.adapter_digest.as_str() {
        bail!(
            "process adapter_digest drift: manifest declares {}, actual helper/config digest is {}",
            manifest.adapter_digest.as_str(),
            actual_digest
        );
    }
    Ok(())
}

fn process_adapter_binding(
    resolved: &ResolvedProcessRun,
    execution: &ProcessExecutionContract,
) -> Result<Value> {
    let mut file_arguments = Vec::new();
    for index in 0..execution.args.len() {
        let argument = &execution.args[index];
        if index > 0 && matches!(execution.args[index - 1].as_str(), "-c" | "--eval" | "-e") {
            continue;
        }
        if should_bind_process_file_argument(&resolved.cwd, argument) {
            file_arguments.push(resolved.file_argument_identity(index)?);
        }
    }
    Ok(json!({
        "schema_version": "planr.process_adapter.binding.v1",
        "execution_contract": execution,
        "file_arguments": file_arguments,
    }))
}

fn should_bind_process_file_argument(cwd: &Path, argument: &str) -> bool {
    if argument.contains("://") {
        return false;
    }
    let path = Path::new(argument);
    if path.is_absolute() {
        return true;
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return true;
    }
    if argument.contains('/') || argument.contains('\\') {
        return true;
    }
    cwd.join(path).symlink_metadata().is_ok()
}

fn ensure_input_instance_matches_registration(
    supplied: &VerificationCapabilityInstance,
    registered: &VerificationCapabilityInstance,
) -> Result<()> {
    if serde_json::to_value(supplied)? != serde_json::to_value(registered)? {
        bail!("capability instance does not match registered capability snapshot");
    }
    Ok(())
}

fn ensure_registered_execution_contract_digest(
    conn: &Connection,
    instance_id: &EvidenceId,
    execution_contract_digest: &str,
) -> Result<()> {
    let host_fingerprint_json: String = conn.query_row(
        "SELECT host_fingerprint_json FROM verification_capability_instances WHERE id = ?1",
        [instance_id.as_str()],
        |row| row.get(0),
    )?;
    let host_fingerprint: Value = serde_json::from_str(&host_fingerprint_json)
        .context("decoding capability instance host fingerprint")?;
    let Some(registered_digest) = host_fingerprint
        .get("execution_contract_digest")
        .and_then(Value::as_str)
    else {
        bail!("capability instance is missing canonical execution contract digest");
    };
    if registered_digest != execution_contract_digest {
        bail!("execution contract does not match registered adapter contract");
    }
    Ok(())
}

fn ensure_available_process_instance(instance: &VerificationCapabilityInstance) -> Result<()> {
    if instance.availability.status != super::CapabilityAvailabilityStatus::Available {
        bail!("capability instance is not available");
    }
    if instance.surface != "local-process" {
        bail!("configured process adapters require local-process surface");
    }
    Ok(())
}

fn ensure_contract_compatibility(
    obligation: &ProofObligation,
    instance: &VerificationCapabilityInstance,
    execution: &ProcessExecutionContract,
) -> Result<()> {
    let structured = requires_structured_observation_results(execution);
    if execution.payload_schema.schema_ref != instance.observed_payload_contract.schema_ref {
        bail!("execution contract payload schema does not match registered capability instance");
    }
    if !structured
        && !supports_observation_type(instance, execution.payload_schema.observation_type.as_str())
    {
        bail!("execution contract observation type is not supported by capability instance");
    }
    for observation in &obligation.observations {
        if !supports_observation_type(instance, observation.observation_type.as_str()) {
            bail!("proof obligation observation type is not supported by capability instance");
        }
        if !structured
            && let Some(payload_schema) = &observation.payload_schema
            && payload_schema.schema_ref != instance.observed_payload_contract.schema_ref
        {
            bail!("proof obligation payload schema does not match capability instance");
        }
    }
    Ok(())
}

fn supports_observation_type(
    instance: &VerificationCapabilityInstance,
    observation_type: &str,
) -> bool {
    instance
        .observed_payload_contract
        .observation_types
        .iter()
        .any(|observed| observed.as_str() == observation_type)
}

fn ensure_runtime_bindings(
    obligation: &ProofObligation,
    instance: &VerificationCapabilityInstance,
    target: &TargetBinding,
    environment: &EnvironmentBinding,
    fixture_disclosure: &FixtureDisclosure,
) -> Result<()> {
    if serde_json::to_value(environment)? != serde_json::to_value(&instance.environment)? {
        bail!("runtime environment does not match registered capability instance");
    }
    let target_value = serde_json::to_value(target)?;
    for observation in &obligation.observations {
        if observation.target != target_value {
            bail!("proof obligation target does not match runtime target binding");
        }
    }
    ensure_fixture_disclosure_allowed(&obligation.fixture_policy, fixture_disclosure)
}

fn ensure_fixture_disclosure_allowed(
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
        bail!("fixture disclosure uses fixtures disallowed by proof obligation policy");
    }
    if disclosure.mocks_used && !mocks_allowed {
        bail!("fixture disclosure uses mocks disallowed by proof obligation policy");
    }
    if disclosure_required {
        if disclosure.fixtures_used
            && disclosure
                .fixture_refs
                .as_ref()
                .is_none_or(|refs| refs.is_empty())
        {
            bail!("fixture disclosure must name fixture refs when fixtures are used");
        }
        if disclosure.mocks_used
            && disclosure
                .mock_refs
                .as_ref()
                .is_none_or(|refs| refs.is_empty())
        {
            bail!("fixture disclosure must name mock refs when mocks are used");
        }
    }
    Ok(())
}

const PLANR_EVIDENCE_TARGET_JSON_ENV: &str = "PLANR_EVIDENCE_TARGET_JSON";
const PLANR_EVIDENCE_ENVIRONMENT_JSON_ENV: &str = "PLANR_EVIDENCE_ENVIRONMENT_JSON";
const PLANR_EVIDENCE_EXECUTION_CONTRACT_DIGEST_ENV: &str =
    "PLANR_EVIDENCE_EXECUTION_CONTRACT_DIGEST";

fn evidence_adapter_env(
    caller_env: &BTreeMap<String, String>,
    target: &TargetBinding,
    environment: &EnvironmentBinding,
    execution_contract_digest: &str,
) -> Result<BTreeMap<String, String>> {
    for reserved in [
        PLANR_EVIDENCE_TARGET_JSON_ENV,
        PLANR_EVIDENCE_ENVIRONMENT_JSON_ENV,
        PLANR_EVIDENCE_EXECUTION_CONTRACT_DIGEST_ENV,
    ] {
        if caller_env.contains_key(reserved) {
            bail!("evidence adapter env overrides reserved Planr binding key {reserved}");
        }
    }
    let mut env = caller_env.clone();
    env.insert(
        PLANR_EVIDENCE_TARGET_JSON_ENV.to_string(),
        serde_json::to_value(target)?.to_string(),
    );
    env.insert(
        PLANR_EVIDENCE_ENVIRONMENT_JSON_ENV.to_string(),
        serde_json::to_value(environment)?.to_string(),
    );
    env.insert(
        PLANR_EVIDENCE_EXECUTION_CONTRACT_DIGEST_ENV.to_string(),
        execution_contract_digest.to_string(),
    );
    Ok(env)
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProcessRun {
    pub(crate) cwd: PathBuf,
    pub(crate) argv: Vec<String>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) command_identity: Value,
}

impl ResolvedProcessRun {
    fn resolve(
        repository_root: &Path,
        execution: &ProcessExecutionContract,
        env_overrides: &BTreeMap<String, String>,
    ) -> Result<Self> {
        validate_executable_name(&execution.executable)?;
        validate_args(&execution.args)?;
        validate_env(env_overrides)?;
        let cwd = if let Some(working_directory) = execution.working_directory.as_deref() {
            contained_repository_path(repository_root, working_directory)?
        } else {
            canonical_repository_root(repository_root)?
        };
        let path_value =
            env::var("PATH").context("PATH is required to resolve process executable")?;
        let executable = resolve_executable(&execution.executable, &path_value)?;
        let mut argv = Vec::with_capacity(execution.args.len() + 1);
        argv.push(executable.path.to_string_lossy().to_string());
        argv.extend(execution.args.clone());
        let mut env = env_overrides.clone();
        env.insert("PATH".to_string(), path_value.clone());
        let command_identity = json!({
            "kind": "command",
            "command": &argv,
            "cwd": cwd.to_string_lossy(),
            "env_digest": sha256_json_digest(&json!({
                "env": env,
                "executable": {
                    "requested": execution.executable,
                    "path": executable.path.to_string_lossy(),
                    "path_digest": executable.path_digest,
                    "content_digest": executable.content_digest,
                },
                "path_digest": sha256_prefixed_bytes(path_value.as_bytes()),
            }))?,
        });
        Ok(Self {
            cwd,
            argv,
            env,
            command_identity,
        })
    }

    fn unresolved(
        repository_root: &Path,
        execution: &ProcessExecutionContract,
        env_overrides: &BTreeMap<String, String>,
        reason: &str,
    ) -> Result<Self> {
        validate_executable_name(&execution.executable)?;
        validate_args(&execution.args)?;
        validate_env(env_overrides)?;
        let cwd = if let Some(working_directory) = execution.working_directory.as_deref() {
            contained_repository_path(repository_root, working_directory)?
        } else {
            canonical_repository_root(repository_root)?
        };
        let path_value =
            env::var("PATH").context("PATH is required to resolve process executable")?;
        let mut argv = Vec::with_capacity(execution.args.len() + 1);
        argv.push(execution.executable.clone());
        argv.extend(execution.args.clone());
        let mut env = env_overrides.clone();
        env.insert("PATH".to_string(), path_value.clone());
        let command_identity = json!({
            "kind": "command",
            "command": &argv,
            "cwd": cwd.to_string_lossy(),
            "resolution": {
                "status": "unavailable",
                "error": reason,
            },
            "env_digest": sha256_json_digest(&json!({
                "env": env,
                "executable": {
                    "requested": execution.executable,
                    "error": reason,
                },
                "path_digest": sha256_prefixed_bytes(path_value.as_bytes()),
            }))?,
        });
        Ok(Self {
            cwd,
            argv,
            env,
            command_identity,
        })
    }

    fn env_for_process(&self) -> Vec<(&str, String)> {
        self.env
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect()
    }

    pub(crate) fn file_argument_identity(&self, argument_index: usize) -> Result<Value> {
        let argument = self
            .argv
            .get(argument_index + 1)
            .with_context(|| format!("process argument {argument_index} is missing"))?;
        validate_file_argument_path(argument)?;
        let candidate = self.cwd.join(argument);
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("canonicalizing process file argument {argument}"))?;
        if !canonical.starts_with(&self.cwd) {
            bail!("process file argument {argument} escapes the resolved command cwd");
        }
        if !canonical.is_file() {
            bail!("process file argument {argument} must resolve to a file");
        }
        let bytes = fs::read(&canonical)
            .with_context(|| format!("reading process file argument {}", canonical.display()))?;
        let relative = canonical
            .strip_prefix(&self.cwd)
            .with_context(|| {
                format!(
                    "computing path of process file argument {} relative to {}",
                    canonical.display(),
                    self.cwd.display()
                )
            })?
            .to_string_lossy()
            .to_string();
        Ok(json!({
            "argument_index": argument_index,
            "argument": argument,
            "resolved_relative_to": "command_cwd",
            "cwd": self.cwd.to_string_lossy(),
            "path": canonical.to_string_lossy(),
            "cwd_relative_path": relative,
            "path_digest": sha256_prefixed_bytes(canonical.to_string_lossy().as_bytes()),
            "content_digest": sha256_prefixed_bytes(&bytes),
        }))
    }
}

fn validate_file_argument_path(argument: &str) -> Result<()> {
    if argument.is_empty() {
        bail!("process file argument must be non-empty");
    }
    if argument.contains('\0') {
        bail!("process file argument must not contain NUL bytes");
    }
    let path = Path::new(argument);
    if path.is_absolute() {
        bail!("process file argument must be relative to the resolved command cwd");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("process file argument must stay within the resolved command cwd");
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum ResolvedProcessRunResolution {
    Runnable(ResolvedProcessRun),
    Unavailable(ResolvedProcessRun, String),
}

impl ResolvedProcessRunResolution {
    fn run(&self) -> &ResolvedProcessRun {
        match self {
            Self::Runnable(resolved) | Self::Unavailable(resolved, _) => resolved,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedExecutable {
    path: PathBuf,
    path_digest: String,
    content_digest: String,
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

fn validate_args(args: &[String]) -> Result<()> {
    if args.iter().any(|arg| arg.contains('\0')) {
        bail!("process arguments must not contain NUL bytes");
    }
    Ok(())
}

fn validate_env(env: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in env {
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            bail!("environment names must use uppercase ASCII, digits, and underscore");
        }
        if value.contains('\0') {
            bail!("environment values must not contain NUL bytes");
        }
    }
    Ok(())
}

fn canonical_repository_root(repository_root: &Path) -> Result<PathBuf> {
    let canonical = repository_root.canonicalize().with_context(|| {
        format!(
            "canonicalizing repository root {}",
            repository_root.display()
        )
    })?;
    if !canonical.is_dir() {
        bail!("repository root must be a directory");
    }
    Ok(canonical)
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
    let canonical_root = canonical_repository_root(repository_root)?;
    let path = canonical_root.join(relative);
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("canonicalizing repository path {}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("repository path escapes repository root");
    }
    Ok(canonical_path)
}

fn resolve_executable(executable: &str, path: &str) -> Result<ResolvedExecutable> {
    for directory in env::split_paths(path) {
        let candidate = directory.join(executable);
        if !candidate.is_file() {
            continue;
        }
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("canonicalizing executable {}", candidate.display()))?;
        let content = fs::read(&canonical)
            .with_context(|| format!("reading executable {}", canonical.display()))?;
        return Ok(ResolvedExecutable {
            path_digest: sha256_prefixed_bytes(canonical.to_string_lossy().as_bytes()),
            content_digest: sha256_prefixed_bytes(&content),
            path: canonical,
        });
    }
    bail!("process executable {executable} was not found in captured PATH");
}

fn is_missing_process_executable(error: &anyhow::Error) -> bool {
    error.to_string().starts_with("process executable ")
        && error
            .to_string()
            .ends_with(" was not found in captured PATH")
}

struct AdapterProcessResult {
    status: AttemptStatus,
    exit: Value,
    stdout_digest: String,
    stderr_digest: String,
    raw_result: Value,
    artifacts: Vec<Value>,
    output_bounds: Value,
}

fn attempt_result(
    output: Result<BoundedProcessOutput>,
    resolved: &ResolvedProcessRun,
) -> Result<AdapterProcessResult> {
    match output {
        Ok(output) => {
            let status = if output.timed_out {
                AttemptStatus::TimedOut
            } else if output.interrupted {
                AttemptStatus::Aborted
            } else if output.exit_code == Some(0) {
                AttemptStatus::Passed
            } else {
                AttemptStatus::Failed
            };
            let boundary_signal = if status == AttemptStatus::Failed {
                adapter_boundary_signal_from_process_output(
                    Some(output.stdout_excerpt.as_str()),
                    Some(output.stderr_excerpt.as_str()),
                )
            } else {
                None
            };
            let error = match status {
                AttemptStatus::Passed => Value::Null,
                AttemptStatus::Failed => Value::String(
                    boundary_signal
                        .map(AdapterBoundarySignal::as_str)
                        .unwrap_or("process_exit_nonzero")
                        .to_string(),
                ),
                AttemptStatus::TimedOut => Value::String("timed_out".to_string()),
                AttemptStatus::Aborted => Value::String("aborted".to_string()),
                _ => Value::Null,
            };
            let signal = if status == AttemptStatus::Aborted {
                Value::String("cancelled".to_string())
            } else {
                Value::Null
            };
            let exit = json!({
                "exit_code": output.exit_code,
                "signal": signal,
                "error": error
            });
            let raw_result = json!({
                "kind": "process_output",
                "digest": sha256_json_digest(&json!({
                    "stdout_digest": output.stdout_digest,
                    "stderr_digest": output.stderr_digest,
                    "exit": exit,
                }))?,
                "argv": output.argv,
                "cwd": resolved.cwd.to_string_lossy(),
                "exit": exit,
                "stdout_digest": output.stdout_digest,
                "stderr_digest": output.stderr_digest,
                "stdout_excerpt": output.stdout_excerpt,
                "stderr_excerpt": output.stderr_excerpt,
                "stdout_bytes": output.stdout_bytes,
                "stderr_bytes": output.stderr_bytes,
                "stdout_truncated": output.stdout_truncated,
                "stderr_truncated": output.stderr_truncated,
            });
            let output_bounds = json!({
                "stdout_bytes": output.stdout_bytes,
                "stderr_bytes": output.stderr_bytes,
                "stdout_truncated": output.stdout_truncated,
                "stderr_truncated": output.stderr_truncated,
            });
            let artifacts = vec![
                json!({
                    "id": "stdout",
                    "kind": "stdout",
                    "digest": output.stdout_digest,
                    "inline_excerpt": output.stdout_excerpt
                }),
                json!({
                    "id": "stderr",
                    "kind": "stderr",
                    "digest": output.stderr_digest,
                    "inline_excerpt": output.stderr_excerpt
                }),
            ];
            Ok(AdapterProcessResult {
                status,
                exit,
                stdout_digest: output.stdout_digest,
                stderr_digest: output.stderr_digest,
                raw_result,
                artifacts,
                output_bounds,
            })
        }
        Err(error) => {
            if let Some(process_error) = error.downcast_ref::<BoundedProcessError>()
                && process_error.is_output_limit_exceeded()
                && let Some(output) = process_error.output()
            {
                return process_error_result(output.clone(), resolved);
            }
            unavailable_process_error_result(error.to_string(), resolved)
        }
    }
}

fn unavailable_process_error_result(
    reason: String,
    resolved: &ResolvedProcessRun,
) -> Result<AdapterProcessResult> {
    let empty_digest = sha256_prefixed_bytes(&[]);
    let exit = json!({
        "exit_code": null,
        "signal": null,
        "error": "unavailable"
    });
    let raw_result = json!({
        "kind": "process_error",
        "digest": empty_digest,
        "argv": resolved.argv,
        "cwd": resolved.cwd.to_string_lossy(),
        "exit": exit,
        "error_reason": reason,
        "stdout_digest": empty_digest,
        "stderr_digest": empty_digest,
        "stdout_bytes": 0,
        "stderr_bytes": 0,
        "stdout_truncated": false,
        "stderr_truncated": false
    });
    let output_bounds = json!({
        "stdout_bytes": 0,
        "stderr_bytes": 0,
        "stdout_truncated": false,
        "stderr_truncated": false,
    });
    Ok(AdapterProcessResult {
        status: AttemptStatus::Unavailable,
        exit,
        stdout_digest: empty_digest.clone(),
        stderr_digest: empty_digest,
        raw_result,
        artifacts: vec![],
        output_bounds,
    })
}

fn process_error_result(
    output: BoundedProcessOutput,
    resolved: &ResolvedProcessRun,
) -> Result<AdapterProcessResult> {
    let stdout_digest = output.stdout_digest.clone();
    let stderr_digest = output.stderr_digest.clone();
    let stdout_excerpt = output.stdout_excerpt.clone();
    let stderr_excerpt = output.stderr_excerpt.clone();
    let exit = json!({
        "exit_code": output.exit_code,
        "signal": null,
        "error": "output_limit_exceeded"
    });
    let raw_result = json!({
        "kind": "process_error",
        "digest": sha256_json_digest(&json!({
            "stdout_digest": stdout_digest,
            "stderr_digest": stderr_digest,
            "exit": exit,
            "error_reason": "output_limit_exceeded",
        }))?,
        "argv": output.argv,
        "cwd": resolved.cwd.to_string_lossy(),
        "exit": exit,
        "error_reason": "output_limit_exceeded",
        "stdout_digest": stdout_digest,
        "stderr_digest": stderr_digest,
        "stdout_excerpt": stdout_excerpt,
        "stderr_excerpt": stderr_excerpt,
        "stdout_bytes": output.stdout_bytes,
        "stderr_bytes": output.stderr_bytes,
        "stdout_truncated": output.stdout_truncated,
        "stderr_truncated": output.stderr_truncated
    });
    let output_bounds = json!({
        "stdout_bytes": output.stdout_bytes,
        "stderr_bytes": output.stderr_bytes,
        "stdout_truncated": output.stdout_truncated,
        "stderr_truncated": output.stderr_truncated,
    });
    let artifacts = vec![
        json!({
            "id": "stdout",
            "kind": "stdout",
            "digest": stdout_digest,
            "inline_excerpt": stdout_excerpt
        }),
        json!({
            "id": "stderr",
            "kind": "stderr",
            "digest": stderr_digest,
            "inline_excerpt": stderr_excerpt
        }),
    ];
    Ok(AdapterProcessResult {
        status: AttemptStatus::Unavailable,
        exit,
        stdout_digest,
        stderr_digest,
        raw_result,
        artifacts,
        output_bounds,
    })
}

fn observation_results(
    obligation: &ProofObligation,
    status: AttemptStatus,
    raw_result: &Value,
    structured_observation_results: Option<&BTreeMap<String, Map<String, Value>>>,
) -> Result<Vec<ObservationResult>> {
    obligation
        .observations
        .iter()
        .map(|observation| {
            Ok(ObservationResult {
                requirement_id: observation.id.clone(),
                observation_type: observation.observation_type.clone(),
                outcome: status,
                predicate: value_object_or_wrapped(observation.expected.clone()),
                actual: observation_actual(raw_result, observation, structured_observation_results),
            })
        })
        .collect()
}

fn observation_actual(
    raw_result: &Value,
    observation: &super::model::ObservationRequirement,
    structured_observation_results: Option<&BTreeMap<String, Map<String, Value>>>,
) -> Map<String, Value> {
    if let Some(actuals) = structured_observation_results
        && let Some(actual) = actuals.get(observation.id.as_str())
    {
        return actual.clone();
    }
    value_object_or_wrapped(raw_result.clone())
}

fn requires_structured_observation_results(execution: &ProcessExecutionContract) -> bool {
    execution.payload_schema.schema_ref == "schema://planr.structured_observation_results.v1"
}

struct StructuredObservationContext<'a> {
    target: &'a TargetBinding,
    environment: &'a EnvironmentBinding,
    fixture_disclosure: &'a FixtureDisclosure,
    repository_root: &'a Path,
}

fn strict_structured_observation_results(
    obligation: &ProofObligation,
    execution: &ProcessExecutionContract,
    observation_payload_json_schemas: &BTreeMap<String, Value>,
    context: StructuredObservationContext<'_>,
    raw_result: &Value,
) -> Result<BTreeMap<String, Map<String, Value>>> {
    if raw_result
        .get("stdout_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        bail!("structured observation results stdout was truncated");
    }
    let stdout = raw_result
        .get("stdout_excerpt")
        .and_then(Value::as_str)
        .context("structured observation results missing stdout")?;
    let parsed: Value = serde_json::from_str(stdout)
        .context("structured observation results must be single JSON")?;
    if parsed.get("schema_version").and_then(Value::as_str)
        != Some("planr.structured_observation_results.v1")
    {
        bail!("structured observation results schema_version mismatch");
    }
    if parsed.get("target") != Some(&serde_json::to_value(context.target)?) {
        bail!("structured observation results target does not match runtime target binding");
    }
    ensure_structured_observed_target_matches(&parsed, context.target)?;
    if parsed.get("environment") != Some(&serde_json::to_value(context.environment)?) {
        bail!(
            "structured observation results environment does not match runtime environment binding"
        );
    }
    if parsed
        .get("execution_contract_digest")
        .and_then(Value::as_str)
        != Some(sha256_json_digest(&serde_json::to_value(execution)?)?.as_str())
    {
        bail!("structured observation results execution contract digest mismatch");
    }
    ensure_structured_fixture_disclosure_matches(
        &parsed,
        context.repository_root,
        context.fixture_disclosure,
    )?;
    let observations = parsed
        .get("observations")
        .and_then(Value::as_array)
        .context("structured observation results missing observations array")?;
    if observations.len() != obligation.observations.len() {
        bail!("structured observation results count does not match proof obligation");
    }
    let mut requirements = BTreeMap::new();
    for observation in &obligation.observations {
        requirements.insert(observation.id.as_str(), observation);
    }
    let mut seen = BTreeSet::new();
    let mut actuals = BTreeMap::new();
    for result in observations {
        let object = result
            .as_object()
            .context("structured observation result entries must be objects")?;
        let allowed = BTreeSet::from(["requirement_id", "type", "actual"]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            bail!(
                "structured observation result has unknown fields: {}",
                unknown.join(",")
            );
        }
        let requirement_id = result
            .get("requirement_id")
            .and_then(Value::as_str)
            .context("structured observation result missing requirement_id")?;
        if !seen.insert(requirement_id.to_string()) {
            bail!("structured observation result duplicates requirement_id {requirement_id}");
        }
        let requirement = requirements.get(requirement_id).with_context(|| {
            format!("structured observation result has unknown requirement_id {requirement_id}")
        })?;
        if result.get("type").and_then(Value::as_str) != Some(requirement.observation_type.as_str())
        {
            bail!("structured observation result type mismatch for {requirement_id}");
        }
        let actual = result
            .get("actual")
            .and_then(Value::as_object)
            .with_context(|| {
                format!("structured observation result actual must be object for {requirement_id}")
            })?;
        let expected_schema_ref = requirement
            .payload_schema
            .as_ref()
            .map(|schema| schema.schema_ref.as_str())
            .unwrap_or(execution.payload_schema.schema_ref.as_str());
        if actual.get("schema_ref").and_then(Value::as_str) != Some(expected_schema_ref) {
            bail!("structured observation result schema_ref mismatch for {requirement_id}");
        }
        if let Some(schema) = observation_payload_json_schemas.get(requirement_id) {
            let validator = jsonschema::draft202012::options()
                .build(schema)
                .with_context(|| {
                    format!(
                        "structured observation payload schema is invalid for {requirement_id} ({expected_schema_ref})"
                    )
                })?;
            let schema_errors = validator
                .iter_errors(&Value::Object(actual.clone()))
                .map(|error| {
                    format!(
                        "{}: {error}",
                        if error.instance_path().as_str().is_empty() {
                            "/"
                        } else {
                            error.instance_path().as_str()
                        }
                    )
                })
                .collect::<Vec<_>>();
            if !schema_errors.is_empty() {
                bail!(
                    "structured observation payload schema mismatch for {requirement_id} ({expected_schema_ref}): {}",
                    schema_errors.join("; ")
                );
            }
        }
        if !actual_satisfies_expected(&Value::Object(actual.clone()), &requirement.expected) {
            bail!(
                "structured observation result actual does not satisfy expected predicate for {requirement_id}"
            );
        }
        actuals.insert(requirement_id.to_string(), actual.clone());
    }
    for requirement in &obligation.observations {
        if !seen.contains(requirement.id.as_str()) {
            bail!(
                "structured observation results missing requirement_id {}",
                requirement.id.as_str()
            );
        }
    }
    Ok(actuals)
}

#[derive(Debug)]
enum OrdinaryObservationError {
    Malformed(String),
    SemanticMismatch(String),
}

fn strict_ordinary_process_observation_results(
    obligation: &ProofObligation,
    payload_json_schema: Option<&Value>,
    raw_result: &Value,
) -> std::result::Result<BTreeMap<String, Map<String, Value>>, OrdinaryObservationError> {
    if raw_result
        .get("stdout_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        return Err(OrdinaryObservationError::Malformed(
            "ordinary process stdout was truncated".to_string(),
        ));
    }
    let stdout = raw_result
        .get("stdout_excerpt")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            OrdinaryObservationError::Malformed(
                "ordinary process result missing stdout".to_string(),
            )
        })?;
    let parsed: Value = serde_json::from_str(stdout).map_err(|error| {
        OrdinaryObservationError::Malformed(format!(
            "ordinary process stdout must be single JSON: {error}"
        ))
    })?;
    if !parsed.is_object() {
        return Err(OrdinaryObservationError::Malformed(
            "ordinary process stdout must be a JSON object".to_string(),
        ));
    }
    if let Some(schema) = payload_json_schema {
        let validator = jsonschema::draft202012::options()
            .build(schema)
            .map_err(|error| {
                OrdinaryObservationError::Malformed(format!(
                    "ordinary process payload schema is invalid: {error}"
                ))
            })?;
        let schema_errors = validator
            .iter_errors(&parsed)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        if !schema_errors.is_empty() {
            return Err(OrdinaryObservationError::Malformed(format!(
                "ordinary process stdout does not match payload schema: {}",
                schema_errors.join("; ")
            )));
        }
    }
    let actual_object = parsed.as_object().cloned().ok_or_else(|| {
        OrdinaryObservationError::Malformed(
            "ordinary process stdout must be a JSON object".to_string(),
        )
    })?;
    let mut actuals = BTreeMap::new();
    for observation in &obligation.observations {
        let actual = Value::Object(actual_object.clone());
        if !actual_satisfies_expected(&actual, &observation.expected) {
            return Err(OrdinaryObservationError::SemanticMismatch(format!(
                "ordinary process actual does not satisfy expected predicate for {}",
                observation.id.as_str()
            )));
        }
        let mut bound_actual = actual_object.clone();
        if let Some(schema) = &observation.payload_schema {
            bound_actual.insert("schema_ref".to_string(), json!(schema.schema_ref));
        }
        actuals.insert(observation.id.as_str().to_string(), bound_actual);
    }
    Ok(actuals)
}

fn ensure_structured_fixture_disclosure_matches(
    parsed: &Value,
    repository_root: &Path,
    disclosure: &FixtureDisclosure,
) -> Result<()> {
    let Some(producer_disclosure) = parsed.get("fixture_disclosure") else {
        bail!("structured observation results missing fixture_disclosure");
    };
    let required = derive_fixture_disclosure_from_producer_output(repository_root, parsed)?;
    if producer_disclosure != &serde_json::to_value(&required)? {
        bail!("structured observation fixture_disclosure does not match verified fixture sources");
    }
    if disclosure.fixtures_used != required.fixtures_used
        || disclosure.mocks_used != required.mocks_used
        || normalized_refs(disclosure.fixture_refs.as_ref())
            != normalized_refs(required.fixture_refs.as_ref())
        || normalized_refs(disclosure.mock_refs.as_ref())
            != normalized_refs(required.mock_refs.as_ref())
    {
        bail!("fixture disclosure does not match structured producer fixture disclosure");
    }
    Ok(())
}

fn normalized_refs(refs: Option<&Vec<String>>) -> Vec<String> {
    refs.cloned().unwrap_or_default()
}

fn derive_fixture_disclosure_from_producer_output(
    repository_root: &Path,
    parsed: &Value,
) -> Result<FixtureDisclosure> {
    let disclosure = parsed
        .get("fixture_disclosure")
        .and_then(Value::as_object)
        .context("structured observation fixture_disclosure must be an object")?;
    let allowed = BTreeSet::from(["fixtures_used", "mocks_used", "fixture_refs", "mock_refs"]);
    let unknown = disclosure
        .keys()
        .filter(|key| !allowed.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        bail!(
            "structured observation fixture_disclosure has unknown fields: {}",
            unknown.join(",")
        );
    }
    let fixtures_used = disclosure
        .get("fixtures_used")
        .and_then(Value::as_bool)
        .context("structured observation fixture_disclosure fixtures_used must be a boolean")?;
    let mocks_used = disclosure
        .get("mocks_used")
        .and_then(Value::as_bool)
        .context("structured observation fixture_disclosure mocks_used must be a boolean")?;
    let fixture_refs = derive_structured_fixture_refs(repository_root, parsed, "fixture_sources")?;
    let mock_refs = derive_structured_fixture_refs(repository_root, parsed, "mock_sources")?;
    if fixtures_used && fixture_refs.as_ref().is_none_or(Vec::is_empty) {
        bail!("structured observation fixture_sources are required when fixtures are used");
    }
    if !fixtures_used && fixture_refs.as_ref().is_some_and(|refs| !refs.is_empty()) {
        bail!(
            "structured observation fixture_sources require fixture_disclosure.fixtures_used=true"
        );
    }
    if mocks_used && mock_refs.as_ref().is_none_or(Vec::is_empty) {
        bail!("structured observation mock_sources are required when mocks are used");
    }
    if !mocks_used && mock_refs.as_ref().is_some_and(|refs| !refs.is_empty()) {
        bail!("structured observation mock_sources require fixture_disclosure.mocks_used=true");
    }
    Ok(FixtureDisclosure {
        fixtures_used,
        mocks_used,
        fixture_refs,
        mock_refs,
    })
}

fn derive_structured_fixture_refs(
    repository_root: &Path,
    parsed: &Value,
    field: &'static str,
) -> Result<Option<Vec<String>>> {
    let Some(sources) = parsed.get(field) else {
        return Ok(None);
    };
    let array = sources
        .as_array()
        .with_context(|| format!("structured observation {field} must be an array"))?;
    let mut refs = Vec::with_capacity(array.len());
    for source in array {
        let object = source
            .as_object()
            .with_context(|| format!("structured observation {field} entries must be objects"))?;
        let allowed = BTreeSet::from(["ref", "path", "digest"]);
        let unknown = object
            .keys()
            .filter(|key| !allowed.contains(key.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            bail!(
                "structured observation {field} entry has unknown fields: {}",
                unknown.join(",")
            );
        }
        let reference = object
            .get("ref")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .with_context(|| format!("structured observation {field}[].ref is required"))?;
        let relative_path = object
            .get("path")
            .and_then(Value::as_str)
            .with_context(|| format!("structured observation {field}[].path is required"))?;
        let declared_digest = verify_structured_fixture_source_digest(
            repository_root,
            relative_path,
            object.get("digest"),
        )?;
        if !reference.ends_with(&declared_digest) {
            bail!("structured observation {field}[].ref must be bound to its verified digest");
        }
        refs.push(reference.to_string());
    }
    Ok(Some(refs))
}

fn verify_structured_fixture_source_digest(
    repository_root: &Path,
    relative_path: &str,
    digest: Option<&Value>,
) -> Result<String> {
    if relative_path.is_empty()
        || Path::new(relative_path).is_absolute()
        || Path::new(relative_path).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("structured observation fixture source path must be a contained relative path");
    }
    let declared_digest = digest
        .and_then(Value::as_str)
        .context("structured observation fixture source digest is required")?;
    Sha256Digest::parse(declared_digest.to_string())?;
    let root = repository_root.canonicalize().with_context(|| {
        format!(
            "canonicalizing fixture provenance repository root {}",
            repository_root.display()
        )
    })?;
    let path = root
        .join(relative_path)
        .canonicalize()
        .with_context(|| format!("canonicalizing structured fixture source {relative_path}"))?;
    if !path.starts_with(&root) {
        bail!("structured observation fixture source path escapes repository root");
    }
    let actual_digest = sha256_prefixed_bytes(&fs::read(&path)?);
    if actual_digest != declared_digest {
        bail!("structured observation fixture source digest mismatch for {relative_path}");
    }
    Ok(declared_digest.to_string())
}

fn ensure_structured_observed_target_matches(parsed: &Value, target: &TargetBinding) -> Result<()> {
    let observed = parsed
        .get("observed_target")
        .and_then(Value::as_object)
        .context("structured observation results missing observed_target")?;
    if observed.get("kind").and_then(Value::as_str) != Some(target.kind.as_str()) {
        bail!(
            "structured observation results observed_target kind does not match runtime target binding"
        );
    }
    if let Some(expected_uri) = target.uri.as_deref()
        && observed.get("initial_uri").and_then(Value::as_str) != Some(expected_uri)
    {
        bail!(
            "structured observation results observed_target initial_uri does not match runtime target binding"
        );
    }
    if target.uri.is_some() && observed.get("final_uri").and_then(Value::as_str).is_none() {
        bail!("structured observation results observed_target final_uri is missing");
    }
    Ok(())
}

fn mark_structured_observation_failure(
    process_result: &mut AdapterProcessResult,
    error: String,
) -> Result<()> {
    process_result.status = AttemptStatus::Failed;
    process_result.exit = json!({
        "exit_code": 1,
        "signal": null,
        "error": "verifier_failed",
    });
    if let Some(raw) = process_result.raw_result.as_object_mut() {
        raw.insert("exit".to_string(), process_result.exit.clone());
        raw.insert(
            "planr_adapter_gap_reasons".to_string(),
            json!(["verifier_failed"]),
        );
        raw.insert(
            "structured_observation_error".to_string(),
            Value::String(error),
        );
        let digest = sha256_json_digest(&json!({
            "stdout_digest": process_result.stdout_digest,
            "stderr_digest": process_result.stderr_digest,
            "exit": process_result.exit,
            "structured_observation_error": raw.get("structured_observation_error"),
        }))?;
        raw.insert("digest".to_string(), Value::String(digest));
    }
    Ok(())
}

fn mark_ordinary_observation_verifier_failure(
    process_result: &mut AdapterProcessResult,
    error: String,
) -> Result<()> {
    process_result.status = AttemptStatus::Failed;
    process_result.exit = json!({
        "exit_code": 1,
        "signal": null,
        "error": "verifier_failed",
    });
    if let Some(raw) = process_result.raw_result.as_object_mut() {
        raw.insert("exit".to_string(), process_result.exit.clone());
        raw.insert(
            "planr_adapter_gap_reasons".to_string(),
            json!(["verifier_failed"]),
        );
        raw.insert(
            "ordinary_observation_error".to_string(),
            Value::String(error),
        );
        let digest = sha256_json_digest(&json!({
            "result": raw,
            "status": process_result.status.as_str(),
            "exit": process_result.exit,
        }))?;
        raw.insert("raw_result_digest".to_string(), json!(digest));
    }
    Ok(())
}

fn mark_semantic_observation_mismatch(
    process_result: &mut AdapterProcessResult,
    error: String,
) -> Result<()> {
    process_result.status = AttemptStatus::Failed;
    process_result.exit = json!({
        "exit_code": 1,
        "signal": null,
        "error": "target_mismatch",
    });
    if let Some(raw) = process_result.raw_result.as_object_mut() {
        raw.insert("exit".to_string(), process_result.exit.clone());
        raw.insert(
            "planr_adapter_gap_reasons".to_string(),
            json!(["target_mismatch"]),
        );
        raw.insert(
            "ordinary_observation_error".to_string(),
            Value::String(error),
        );
        let digest = sha256_json_digest(&json!({
            "stdout_digest": process_result.stdout_digest,
            "stderr_digest": process_result.stderr_digest,
            "exit": process_result.exit,
            "ordinary_observation_error": raw.get("ordinary_observation_error"),
        }))?;
        raw.insert("digest".to_string(), Value::String(digest));
    }
    Ok(())
}

fn actual_satisfies_expected(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(expected_map) => {
            let Some(actual_map) = actual.as_object() else {
                return false;
            };
            expected_map.iter().all(|(key, expected_value)| {
                actual_map.get(key).is_some_and(|actual_value| {
                    actual_satisfies_expected(actual_value, expected_value)
                })
            })
        }
        Value::Array(expected_items) => actual.as_array().is_some_and(|actual_items| {
            actual_items.len() == expected_items.len()
                && actual_items
                    .iter()
                    .zip(expected_items)
                    .all(|(actual_item, expected_item)| {
                        actual_satisfies_expected(actual_item, expected_item)
                    })
        }),
        _ => actual == expected,
    }
}

fn value_object_or_wrapped(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => Map::from_iter([("value".to_string(), other)]),
    }
}

#[derive(Debug, Clone)]
struct ResolvedRetryLineage {
    retry_of: Option<EvidenceId>,
    attempt_number: u32,
    max_attempts: u32,
    previous_attempt_ids: Vec<EvidenceId>,
    value: Value,
}

fn resolve_retry_lineage(
    conn: &Connection,
    project_id: &str,
    obligation_id: &EvidenceId,
    capability_instance_id: &EvidenceId,
    retry_of: Option<&EvidenceId>,
    attempt_index: u32,
    max_attempts: u32,
) -> Result<ResolvedRetryLineage> {
    if max_attempts == 0 {
        bail!("retry max_attempts must be at least one");
    }
    let Some(retry_of) = retry_of else {
        if attempt_index != 0 {
            bail!("initial evidence attempt must use attempt_index 0");
        }
        let value = json!({
            "attempt_number": 1,
            "max_attempts": max_attempts,
            "previous_attempt_ids": [],
        });
        return Ok(ResolvedRetryLineage {
            retry_of: None,
            attempt_number: 1,
            max_attempts,
            previous_attempt_ids: vec![],
            value,
        });
    };
    let predecessor = conn
        .query_row(
            "SELECT project_id, obligation_id, capability_instance_id, attempt_json
             FROM evidence_attempts
             WHERE id = ?1",
            [retry_of.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .with_context(|| format!("retry predecessor {} was not found", retry_of.as_str()))?;
    if predecessor.0 != project_id {
        bail!("retry predecessor belongs to a different project");
    }
    if predecessor.1 != obligation_id.as_str() {
        bail!("retry predecessor belongs to a different obligation");
    }
    if predecessor.2 != capability_instance_id.as_str() {
        bail!("retry predecessor belongs to a different capability instance");
    }
    let predecessor_attempt: Value =
        serde_json::from_str(&predecessor.3).context("decoding retry predecessor attempt_json")?;
    let predecessor_lineage = predecessor_attempt
        .get("retry_lineage")
        .context("retry predecessor is missing retry_lineage")?;
    let predecessor_attempt_number = predecessor_lineage
        .get("attempt_number")
        .and_then(Value::as_u64)
        .context("retry predecessor is missing attempt_number")?;
    let expected_attempt_number = predecessor_attempt_number
        .checked_add(1)
        .context("retry predecessor attempt_number overflowed")?;
    if u64::from(attempt_index) + 1 != expected_attempt_number {
        bail!("retry attempt_index must be predecessor attempt_index plus one");
    }
    let mut previous_attempt_ids = predecessor_lineage
        .get("previous_attempt_ids")
        .and_then(Value::as_array)
        .context("retry predecessor is missing previous_attempt_ids")?
        .iter()
        .map(|value| {
            let attempt_id = value
                .as_str()
                .context("retry predecessor previous_attempt_ids must be strings")?;
            EvidenceId::parse(attempt_id).map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    previous_attempt_ids.push(retry_of.clone());
    let attempt_number =
        u32::try_from(expected_attempt_number).context("retry attempt_number does not fit u32")?;
    let predecessor_max_attempts = predecessor_lineage
        .get("max_attempts")
        .and_then(Value::as_u64)
        .context("retry predecessor is missing max_attempts")?;
    let predecessor_max_attempts =
        u32::try_from(predecessor_max_attempts).context("retry max_attempts does not fit u32")?;
    if max_attempts != predecessor_max_attempts {
        bail!("retry max_attempts must match predecessor max_attempts");
    }
    if attempt_number > max_attempts {
        bail!("retry max_attempts exhausted before launch");
    }
    let value = json!({
        "attempt_number": attempt_number,
        "max_attempts": max_attempts,
        "previous_attempt_ids": previous_attempt_ids.iter().map(EvidenceId::as_str).collect::<Vec<_>>(),
    });
    Ok(ResolvedRetryLineage {
        retry_of: Some(retry_of.clone()),
        attempt_number,
        max_attempts,
        previous_attempt_ids,
        value,
    })
}

fn retry_history(lineage: &ResolvedRetryLineage, status: AttemptStatus) -> Vec<Map<String, Value>> {
    let mut entry = Map::new();
    entry.insert("attempt_number".to_string(), json!(lineage.attempt_number));
    entry.insert("max_attempts".to_string(), json!(lineage.max_attempts));
    entry.insert("status".to_string(), json!(status.as_str()));
    entry.insert(
        "previous_attempt_ids".to_string(),
        json!(
            lineage
                .previous_attempt_ids
                .iter()
                .map(EvidenceId::as_str)
                .collect::<Vec<_>>()
        ),
    );
    if let Some(retry_of) = &lineage.retry_of {
        entry.insert("retry_of".to_string(), json!(retry_of.as_str()));
    }
    vec![entry]
}

fn proof_gaps(status: AttemptStatus, exit: &Value, raw_result: &Value) -> Vec<GapReason> {
    let mut adapter_gaps = raw_result
        .get("planr_adapter_gap_reasons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    adapter_gaps.extend(adapter_gap_reasons_from_process_output(
        raw_result
            .get("stdout_excerpt")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        raw_result
            .get("stderr_excerpt")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ));
    if !adapter_gaps.is_empty() {
        return adapter_gaps
            .iter()
            .map(|gap| GapReason::canonicalize(gap))
            .collect();
    }
    match status {
        AttemptStatus::Passed => vec![],
        AttemptStatus::TimedOut => vec![GapReason::TimedOut],
        AttemptStatus::Aborted => vec![GapReason::Aborted],
        AttemptStatus::Unavailable => vec![GapReason::MissingCapability],
        AttemptStatus::Failed => match exit.get("error").and_then(Value::as_str) {
            Some("permission_denied") => vec![GapReason::PermissionDenied],
            Some("sandbox_blocked") => vec![GapReason::SandboxBlocked],
            _ => match exit.get("exit_code").and_then(Value::as_i64) {
                // POSIX shells use 126 when a command is found but cannot be
                // invoked. Treat it as a permission boundary, not a product
                // assertion failure.
                Some(126) => vec![GapReason::PermissionDenied],
                _ => vec![GapReason::ProductFailed],
            },
        },
        _ => vec![GapReason::InconclusiveResult],
    }
}

fn adapter_gap_reasons_from_process_output(stdout: &str, stderr: &str) -> Vec<String> {
    stdout
        .lines()
        .chain(stderr.lines())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value.get("planr_adapter_gap_reasons").cloned())
        .filter_map(|value| {
            value.as_array().map(|gaps| {
                gaps.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
        })
        .next()
        .unwrap_or_default()
}

struct PersistedAttempt<'a> {
    project_id: &'a str,
    obligation_id: &'a EvidenceId,
    attempt: &'a EvidenceAttempt,
    execution_contract_digest: &'a str,
    environment_digest: &'a str,
    retry_predecessor_attempt_id: Option<&'a str>,
}

struct PersistedReceipt<'a> {
    project_id: &'a str,
    obligation_id: &'a EvidenceId,
    attempt: &'a EvidenceAttempt,
    receipt_value: &'a Value,
    receipt_digest: &'a str,
    trusted_binding_value: &'a Value,
    retry_predecessor_attempt_id: Option<&'a str>,
}

fn persist_attempt(conn: &Connection, evidence: PersistedAttempt<'_>) -> Result<()> {
    let exit_code = evidence
        .attempt
        .exit
        .get("exit_code")
        .and_then(Value::as_i64);
    let attempt_json = serde_json::to_string(evidence.attempt)?;
    conn.execute(
        "INSERT INTO evidence_attempts(
          id, project_id, obligation_id, capability_instance_id, attempt_status,
          execution_contract_digest, resolved_command_json, environment_digest,
          retry_predecessor_attempt_id, started_at, completed_at, exit_code, stdout_digest,
          stderr_digest, output_bounds_json, attempt_json, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            evidence.attempt.id.as_str(),
            evidence.project_id,
            evidence.obligation_id.as_str(),
            evidence.attempt.capability_instance_id.as_str(),
            evidence.attempt.status.as_str(),
            evidence.execution_contract_digest,
            serde_json::to_string(&evidence.attempt.resolved_command)?,
            evidence.environment_digest,
            evidence.retry_predecessor_attempt_id,
            evidence.attempt.started_at,
            evidence.attempt.ended_at,
            exit_code,
            evidence.attempt.stdout_digest.as_str(),
            evidence.attempt.stderr_digest.as_str(),
            serde_json::to_string(&evidence.attempt.output_bounds)?,
            attempt_json,
            evidence.attempt.ended_at,
        ],
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "persisting evidence attempt {} status={} exit={} failed: {error}",
            evidence.attempt.id.as_str(),
            evidence.attempt.status.as_str(),
            evidence.attempt.exit
        )
    })?;
    Ok(())
}

fn persist_receipt(conn: &Connection, evidence: PersistedReceipt<'_>) -> Result<()> {
    let supersedes_receipt_id = evidence
        .retry_predecessor_attempt_id
        .map(|attempt_id| {
            conn.query_row(
                "SELECT id FROM evidence_receipts WHERE attempt_id = ?1 LIMIT 1",
                [attempt_id],
                |row| row.get::<_, String>(0),
            )
            .with_context(|| {
                format!("retry predecessor attempt {attempt_id} is missing its receipt")
            })
        })
        .transpose()?;
    conn.execute(
        "INSERT INTO evidence_receipts(
          id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
          trusted_binding_json, observations_json, provenance_json, receipt_json,
          supersedes_receipt_id, created_at
        ) VALUES (?1, ?2, ?3, ?4, 'trusted', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            string_field(evidence.receipt_value, "id")?,
            evidence.project_id,
            evidence.obligation_id.as_str(),
            evidence.attempt.id.as_str(),
            evidence.receipt_digest,
            serde_json::to_string(evidence.trusted_binding_value)?,
            serde_json::to_string(&evidence.receipt_value["observations"])?,
            serde_json::to_string(&evidence.receipt_value["provenance"])?,
            serde_json::to_string(evidence.receipt_value)?,
            supersedes_receipt_id,
            evidence.attempt.ended_at,
        ],
    )?;
    Ok(())
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("receipt field {field} must be a string"))
}

#[derive(Debug, Clone)]
struct RepositorySnapshotMismatch {
    message: String,
    exit_error: &'static str,
    gap_reason: &'static str,
}

#[derive(Debug)]
struct RepositorySnapshotPreCommitMismatch {
    mismatch: RepositorySnapshotMismatch,
}

impl fmt::Display for RepositorySnapshotPreCommitMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.mismatch.message)
    }
}

impl Error for RepositorySnapshotPreCommitMismatch {}

pub(crate) fn run_repository_snapshot_pre_commit_test_hook(repository_root: &Path) -> Result<()> {
    let Some(relative_path) = env::var_os("PLANR_TEST_EVIDENCE_PRE_COMMIT_MUTATE_SOURCE_PATH")
    else {
        return Ok(());
    };
    if !cfg!(debug_assertions) {
        bail!("pre-commit source mutation test hook is only available in debug builds");
    }
    let relative_path = PathBuf::from(relative_path);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("pre-commit source mutation test hook path must stay inside the repository");
    }
    fs::write(
        repository_root.join(relative_path),
        "planr pre-commit source mutation\n",
    )
    .context("running pre-commit source mutation test hook")?;
    Ok(())
}

fn repository_snapshot_mismatch(
    repository_root: &Path,
    expected: &EvidenceRepositorySnapshot,
) -> Result<Option<RepositorySnapshotMismatch>> {
    let current = capture_repository_snapshot(repository_root)
        .map_err(|err| anyhow::anyhow!("capturing repository evidence snapshot: {err}"))?;
    if current.policy != expected.policy {
        return Ok(Some(RepositorySnapshotMismatch {
            message: "repository evidence policy no longer matches launch snapshot before trusted receipt acceptance".to_string(),
            exit_error: "stale_policy",
            gap_reason: GapReason::StalePolicy.as_str(),
        }));
    }
    if current.source != expected.source {
        return Ok(Some(RepositorySnapshotMismatch {
            message: "repository evidence source no longer matches launch snapshot before trusted receipt acceptance".to_string(),
            exit_error: "stale_source",
            gap_reason: GapReason::StaleSource.as_str(),
        }));
    }
    Ok(None)
}

fn mark_repository_snapshot_mismatch(
    process_result: &mut AdapterProcessResult,
    mismatch: &RepositorySnapshotMismatch,
) -> Result<()> {
    process_result.status = AttemptStatus::Failed;
    process_result.exit = json!({
        "exit_code": 1,
        "signal": null,
        "error": mismatch.exit_error,
    });
    if let Some(raw) = process_result.raw_result.as_object_mut() {
        raw.insert("exit".to_string(), process_result.exit.clone());
        raw.insert(
            "planr_adapter_gap_reasons".to_string(),
            json!([mismatch.gap_reason]),
        );
        raw.insert(
            "repository_snapshot_error".to_string(),
            Value::String(mismatch.message.clone()),
        );
        let digest = sha256_json_digest(&json!({
            "result": raw,
            "status": process_result.status.as_str(),
            "exit": process_result.exit,
        }))?;
        raw.insert("raw_result_digest".to_string(), json!(digest));
    }
    Ok(())
}

fn attempt_id(obligation_id: &str, capability_instance_id: &str, started_at: &str) -> String {
    let nonce = Uuid::new_v4();
    let digest = sha256_prefixed_bytes(
        format!("{obligation_id}:{capability_instance_id}:{started_at}:{nonce}").as_bytes(),
    );
    format!("eatt-{}", short_digest(&digest))
}

fn receipt_id(attempt_id: &str) -> String {
    format!("erec-{}", short_digest(attempt_id))
}

fn short_digest(value: &str) -> String {
    value
        .trim_start_matches("sha256:")
        .chars()
        .take(12)
        .collect()
}

fn timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("current UTC time formats as RFC3339")
}

fn limit_to_usize(value: u64) -> usize {
    value.try_into().unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_json::sha256_json_digest_without_top_level_field;
    use crate::evidence::model::{
        CapabilityAvailability, CapabilityAvailabilityStatus, ObservedPayloadContract,
        PermissionState, ReceiptStatus,
    };
    use std::process::Command;
    use tempfile::tempdir;

    const DIGEST_A: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_B: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST_C: &str =
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    type ContractMutation = fn(&mut ProcessExecutionContract);

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE proof_obligations(
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              plan_id TEXT NOT NULL,
              item_id TEXT,
              criterion_id TEXT NOT NULL,
              obligation_version INTEGER NOT NULL,
              title TEXT NOT NULL,
              binding INTEGER NOT NULL,
              observation_requirements_json TEXT NOT NULL,
              fixture_policy_json TEXT NOT NULL,
              freshness_policy_json TEXT NOT NULL,
              assurance_policy_json TEXT NOT NULL,
              policy_digest TEXT NOT NULL,
              config_digest TEXT NOT NULL,
              created_at TEXT NOT NULL
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
              created_at TEXT NOT NULL
            );
            CREATE TABLE verification_capability_manifests(
              id TEXT NOT NULL,
              version TEXT NOT NULL,
              adapter_kind TEXT NOT NULL,
              adapter_digest TEXT NOT NULL,
              manifest_digest TEXT NOT NULL,
              manifest_json TEXT NOT NULL,
              source_path TEXT,
              created_at TEXT NOT NULL,
              PRIMARY KEY(id, version)
            );
            CREATE TABLE evidence_attempts(
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              obligation_id TEXT NOT NULL,
              capability_instance_id TEXT NOT NULL,
              attempt_status TEXT NOT NULL,
              execution_contract_digest TEXT NOT NULL,
              resolved_command_json TEXT NOT NULL,
              environment_digest TEXT NOT NULL,
              retry_predecessor_attempt_id TEXT,
              started_at TEXT NOT NULL,
              completed_at TEXT,
              exit_code INTEGER,
              stdout_digest TEXT,
              stderr_digest TEXT,
              output_bounds_json TEXT NOT NULL DEFAULT '{}',
              attempt_json TEXT NOT NULL,
              created_at TEXT NOT NULL,
              FOREIGN KEY(obligation_id) REFERENCES proof_obligations(id),
              FOREIGN KEY(capability_instance_id) REFERENCES verification_capability_instances(id),
              FOREIGN KEY(retry_predecessor_attempt_id) REFERENCES evidence_attempts(id)
            );
            CREATE TABLE evidence_receipts(
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              obligation_id TEXT NOT NULL,
              attempt_id TEXT NOT NULL,
              receipt_status TEXT NOT NULL,
              receipt_digest TEXT NOT NULL UNIQUE,
              trusted_binding_json TEXT NOT NULL,
              observations_json TEXT NOT NULL,
              provenance_json TEXT NOT NULL,
              receipt_json TEXT NOT NULL,
              supersedes_receipt_id TEXT,
              created_at TEXT NOT NULL,
              FOREIGN KEY(attempt_id) REFERENCES evidence_attempts(id),
              FOREIGN KEY(supersedes_receipt_id) REFERENCES evidence_receipts(id)
            );
            CREATE TRIGGER evidence_attempts_no_update
            BEFORE UPDATE ON evidence_attempts
            BEGIN
              SELECT RAISE(ABORT, 'evidence_attempts are immutable');
            END;
            CREATE TRIGGER evidence_receipts_no_update
            BEFORE UPDATE ON evidence_receipts
            BEGIN
              SELECT RAISE(ABORT, 'evidence_receipts are immutable');
            END;
            "#,
        )
        .unwrap();
        conn
    }

    fn seed(
        conn: &Connection,
        obligation: &ProofObligation,
        instance: &VerificationCapabilityInstance,
        canonical_execution_contract: &ProcessExecutionContract,
    ) {
        seed_obligation_only(conn, obligation);
        seed_instance_only(conn, instance, canonical_execution_contract);
    }

    fn seed_obligation_only(conn: &Connection, obligation: &ProofObligation) {
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, created_at
            ) VALUES (?1, 'p-evidence', 'pln-evidence', 'item-evidence', ?2, 1, ?3, 1, ?4, '{}', '{}', '{}', ?5, ?6, ?7)",
            params![
                obligation.id.as_str(),
                obligation.criterion_id.as_str(),
                obligation.title,
                serde_json::to_string(&obligation.observations).unwrap(),
                DIGEST_A,
                DIGEST_B,
                "2026-07-29T00:00:00Z",
            ],
        )
        .unwrap();
    }

    fn seed_instance_only(
        conn: &Connection,
        instance: &VerificationCapabilityInstance,
        canonical_execution_contract: &ProcessExecutionContract,
    ) {
        let adapter_binding = json!({
            "schema_version": "planr.process_adapter.binding.v1",
            "execution_contract": canonical_execution_contract,
            "file_arguments": [],
        });
        let adapter_digest = sha256_json_digest(&adapter_binding).unwrap();
        let manifest = json!({
            "id": instance.manifest_id.as_str(),
            "schema_version": "evidence.contract.v1",
            "version": "1.0.0",
            "adapter_kind": "process",
            "adapter_digest": adapter_digest,
            "supported_surfaces": ["local-process"],
            "supported_observations": [canonical_execution_contract.payload_schema],
            "supported_interactions": ["process"],
            "supported_artifacts": ["stdout"],
            "runtime_targets": [{"kind": "process", "id": "test"}],
            "provenance_path": "planr_observed_execution",
            "permissions": {"network": "none", "filesystem": "read_workspace"},
            "costs": {},
            "determinism": "deterministic",
            "repeatability": "repeatable",
            "independence": "unit test process adapter",
            "blind_spots": [],
            "availability_probe": {"kind": "process", "execution": canonical_execution_contract},
        });
        let manifest_digest = sha256_json_digest(&manifest).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO verification_capability_manifests(
              id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, source_path, created_at
            ) VALUES (?1, '1.0.0', 'process', ?2, ?3, ?4, '.planr/evidence/adapters/test.json', ?5)",
            params![
                instance.manifest_id.as_str(),
                adapter_digest,
                manifest_digest,
                serde_json::to_string(&manifest).unwrap(),
                instance.captured_at,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verification_capability_instances(
              id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
              availability_status, runtime_target_json, host_fingerprint_json, capability_snapshot_json,
              probe_result_json, created_at
            ) VALUES (?1, ?2, '1.0.0', ?3, 'probe-test', 'available', '{}', ?4, ?5, '{}', ?6)",
            params![
                instance.id.as_str(),
                instance.manifest_id.as_str(),
                instance.manifest_digest.as_str(),
                serde_json::to_string(&json!({
                    "environment": instance.environment,
                    "execution_contract_digest": sha256_json_digest(
                        &serde_json::to_value(canonical_execution_contract).unwrap()
                    ).unwrap(),
                    "execution_contract_source": "repository_adapter_registration"
                }))
                .unwrap(),
                serde_json::to_string(instance).unwrap(),
                instance.captured_at,
            ],
        )
        .unwrap();
    }

    fn insert_prior_attempt(
        conn: &Connection,
        id: &str,
        project_id: &str,
        obligation_id: &str,
        capability_instance_id: &str,
        attempt_index: u32,
        retry_of: Option<&str>,
    ) {
        let attempt_json = json!({
            "id": id,
            "status": "passed",
            "exit": {
                "exit_code": 0,
                "signal": null,
                "error": null
            },
            "retry_lineage": {
                "attempt_number": attempt_index + 1,
                "max_attempts": 3,
                "previous_attempt_ids": retry_of.map(|id| vec![id]).unwrap_or_default()
            }
        });
        conn.execute(
            "INSERT INTO evidence_attempts(
              id, project_id, obligation_id, capability_instance_id, attempt_status,
              execution_contract_digest, resolved_command_json, environment_digest,
              retry_predecessor_attempt_id, started_at, completed_at, exit_code,
              stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
            ) VALUES (
              ?1, ?2, ?3, ?4, 'passed', ?5, '{\"cmd\":\"true\"}', ?6,
              ?7, '2026-07-29T00:00:00Z', '2026-07-29T00:00:01Z', 0,
              ?8, ?8, '{}', ?9, '2026-07-29T00:00:01Z'
            )",
            params![
                id,
                project_id,
                obligation_id,
                capability_instance_id,
                DIGEST_A,
                DIGEST_B,
                retry_of,
                DIGEST_C,
                serde_json::to_string(&attempt_json).unwrap(),
            ],
        )
        .unwrap();
    }

    fn obligation() -> ProofObligation {
        serde_json::from_value(json!({
            "id": "obl-process",
            "schema_version": "evidence.contract.v1",
            "criterion_id": "crit-process",
            "plan_id": "pln-evidence",
            "item_id": "item-evidence",
            "title": "process proof",
            "binding": true,
            "observations": [{
                "id": "obs-process",
                "type": "example.process.stdout",
                "subject": "process stdout",
                "expected": {"contains": "ready"},
                "target": {"kind": "process", "uri": "local://process"}
            }],
            "fixture_policy": {
                "fixtures_allowed": true,
                "mocks_allowed": false,
                "disclosure_required": true
            },
            "freshness_policy": {},
            "assurance_policy": {}
        }))
        .unwrap()
    }

    fn instance() -> VerificationCapabilityInstance {
        VerificationCapabilityInstance {
            id: EvidenceId::parse("capinst-process").unwrap(),
            schema_version: SchemaVersion::v1(),
            manifest_id: EvidenceId::parse("vcap-process").unwrap(),
            manifest_digest: Sha256Digest::parse(DIGEST_A).unwrap(),
            host: "codex".to_string(),
            surface: "local-process".to_string(),
            host_version: "test".to_string(),
            adapter_version: "1.0.0".to_string(),
            environment: EnvironmentBinding {
                kind: "local".to_string(),
                id: EvidenceId::parse("dev-shell").unwrap(),
                digest: Sha256Digest::parse(DIGEST_B).unwrap(),
            },
            permissions: PermissionState {
                network: "none".to_string(),
                filesystem: "read_workspace".to_string(),
                environment: Some("explicit".to_string()),
                secrets: None,
            },
            availability: CapabilityAvailability {
                status: CapabilityAvailabilityStatus::Available,
                reason: None,
            },
            probe_result: serde_json::from_value(json!({
                "probe_execution_id": "probe-test",
                "outcome": "passed",
                "observed_at": "2026-07-29T00:00:00Z",
                "checks": [{"name": "process_probe", "outcome": "passed"}]
            }))
            .unwrap(),
            observed_payload_contract: ObservedPayloadContract {
                schema_ref: "example.process.stdout@v1".to_string(),
                observation_types: vec![
                    super::super::NamespacedIdentifier::parse("example.process.stdout").unwrap(),
                ],
            },
            limitations: vec![],
            captured_at: "2026-07-29T00:00:00Z".to_string(),
        }
    }

    fn execution_contract(
        command: &str,
        args: Vec<&str>,
        timeout_ms: u64,
    ) -> ProcessExecutionContract {
        serde_json::from_value(json!({
            "kind": "process",
            "executable": command,
            "args": args,
            "working_directory": ".",
            "timeout_ms": timeout_ms,
            "stdout_limit_bytes": 1024,
            "stderr_limit_bytes": 1024,
            "payload_schema": {
                "type": "example.process.stdout",
                "schema_ref": "example.process.stdout@v1",
                "schema_digest": DIGEST_C
            }
        }))
        .unwrap()
    }

    fn marker_contract(marker_name: &str) -> ProcessExecutionContract {
        serde_json::from_value(json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", format!("printf launched > {marker_name}")],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 1024,
            "stderr_limit_bytes": 1024,
            "payload_schema": {
                "type": "example.process.stdout",
                "schema_ref": "example.process.stdout@v1",
                "schema_digest": DIGEST_C
            }
        }))
        .unwrap()
    }

    fn shell_contract(script: String) -> ProcessExecutionContract {
        serde_json::from_value(json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", script],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 1024,
            "stderr_limit_bytes": 1024,
            "payload_schema": {
                "type": "example.process.stdout",
                "schema_ref": "example.process.stdout@v1",
                "schema_digest": DIGEST_C
            }
        }))
        .unwrap()
    }

    fn repository_policy_yaml() -> String {
        repository_policy_yaml_with_max_age_seconds(3600)
    }

    fn repository_policy_yaml_with_max_age_seconds(max_age_seconds: u64) -> String {
        let payload_schema = json!({
            "type": "example.process.stdout",
            "schema_ref": "example.process.stdout@v1",
            "schema_digest": DIGEST_C
        });
        let execution_contract = json!({
            "kind": "process",
            "executable": "sh",
            "args": ["-c", "printf '{\"contains\":\"ready\"}'"],
            "working_directory": ".",
            "timeout_ms": 5000,
            "stdout_limit_bytes": 1024,
            "stderr_limit_bytes": 1024,
            "payload_schema": payload_schema
        });
        let mut policy = json!({
            "id": "epolicy-example-process-v1",
            "schema_version": "evidence.contract.v1",
            "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "defaults": {
                "preset_id": "process-proof",
                "binding": true,
                "assurance_level": "standard"
            },
            "named_presets": [{
                "id": "process-proof",
                "schema_version": "evidence.contract.v1",
                "namespace": "example.process.stdout",
                "observations": [{
                    "id": "stdout-ready",
                    "type": "example.process.stdout",
                    "subject": "process stdout",
                    "expected": {"contains": "ready"},
                    "target": {"kind": "process", "uri": "local://process"}
                }]
            }],
            "observation_schema_registrations": [{
                "type": "example.process.stdout",
                "schema_ref": "example.process.stdout@v1",
                "schema_digest": DIGEST_C,
                "owning_namespace": "example.process.stdout"
            }],
            "adapter_registrations": [{
                "manifest_id": "vcap-process",
                "manifest_path": ".planr/evidence/adapters/process.manifest.json",
                "manifest_digest": DIGEST_A,
                "observation_types": ["example.process.stdout"],
                "payload_schemas": [payload_schema],
                "provenance_path": "planr_observed_execution",
                "execution_contract": execution_contract
            }],
            "extension_namespaces": ["example.process.stdout"],
            "trust_policy": {
                "accepted_provenance": ["planr_observed_execution"],
                "min_receipt_status": "trusted",
                "allow_user_attestation": false
            },
            "freshness_policy": {
                "max_age_seconds": max_age_seconds,
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
                    "scope": {"kind": "plan", "id": "pln-evidence"},
                    "policy_digest": DIGEST_A
                }]
            }
        });
        let digest = sha256_json_digest_without_top_level_field(&policy, "policy_digest").unwrap();
        policy["policy_digest"] = json!(digest);
        serde_yaml::to_string(&policy).unwrap()
    }

    fn run_input<'a>(
        root: &'a Path,
        obligation: ProofObligation,
        instance: VerificationCapabilityInstance,
        execution_contract: ProcessExecutionContract,
        cancellation: &'a CancellationToken,
    ) -> ConfiguredProcessRunInput<'a> {
        init_git_repo(root);
        ConfiguredProcessRunInput {
            repository_root: root,
            project_id: "p-evidence",
            obligation,
            capability_instance: instance,
            execution_contract,
            payload_json_schema: None,
            observation_payload_json_schemas: BTreeMap::new(),
            target: TargetBinding {
                kind: "process".to_string(),
                uri: Some("local://process".to_string()),
                digest: None,
                deployment_id: None,
            },
            environment: EnvironmentBinding {
                kind: "local".to_string(),
                id: EvidenceId::parse("dev-shell").unwrap(),
                digest: Sha256Digest::parse(DIGEST_B).unwrap(),
            },
            fixture_disclosure: FixtureDisclosure {
                fixtures_used: true,
                mocks_used: false,
                fixture_refs: Some(vec!["fixture://process".to_string()]),
                mock_refs: None,
            },
            env: BTreeMap::from([("PLANR_TEST_VALUE".to_string(), "ready".to_string())]),
            retry_of: None,
            attempt_index: 0,
            max_attempts: 3,
            cancellation,
        }
    }

    fn init_git_repo(root: &Path) {
        if root.join(".git").exists() {
            return;
        }
        fs::create_dir_all(root.join(".planr")).unwrap();
        let policy_path = root.join(".planr/evidence.yaml");
        if !policy_path.exists() {
            fs::write(policy_path, repository_policy_yaml()).unwrap();
        }
        fs::write(root.join(".planr/repository-snapshot-anchor"), "snapshot\n").unwrap();
        fs::write(root.join(".gitignore"), "*.marker\n").unwrap();
        git(root, &["init"]);
        git(
            root,
            &["config", "user.email", "planr-test@example.invalid"],
        );
        git(root, &["config", "user.name", "Planr Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial evidence snapshot"]);
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn registered_manifest_lookup_uses_the_selected_instance_version() {
        let conn = conn();
        let current_instance = instance();
        let contract =
            execution_contract("sh", vec!["-c", "printf '{\"contains\":\"ready\"}'"], 5000);
        seed_instance_only(&conn, &current_instance, &contract);

        let current_manifest_json: String = conn
            .query_row(
                "SELECT manifest_json FROM verification_capability_manifests
                 WHERE id = ?1 AND version = '1.0.0'",
                [current_instance.manifest_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        let mut legacy_manifest: Value = serde_json::from_str(&current_manifest_json).unwrap();
        legacy_manifest["version"] = json!("0.9.0");
        legacy_manifest["adapter_digest"] = json!(DIGEST_C);
        conn.execute(
            "INSERT INTO verification_capability_manifests(
               id, version, adapter_kind, adapter_digest, manifest_digest,
               manifest_json, source_path, created_at
             ) VALUES (?1, '0.9.0', 'process', ?2, ?2, ?3,
                       '.planr/evidence/adapters/test-legacy.json', ?4)",
            params![
                current_instance.manifest_id.as_str(),
                DIGEST_C,
                serde_json::to_string(&legacy_manifest).unwrap(),
                current_instance.captured_at,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verification_capability_instances(
               id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
               availability_status, runtime_target_json, host_fingerprint_json,
               capability_snapshot_json, probe_result_json, created_at
             ) VALUES ('capinst-process-legacy', ?1, '0.9.0', ?2, 'probe-legacy',
                       'available', '{}', '{}', '{}', '{}', ?3)",
            params![
                current_instance.manifest_id.as_str(),
                DIGEST_C,
                current_instance.captured_at,
            ],
        )
        .unwrap();

        let current = registered_capability_manifest(&conn, &current_instance.id).unwrap();
        let legacy = registered_capability_manifest(
            &conn,
            &EvidenceId::parse("capinst-process-legacy").unwrap(),
        )
        .unwrap();

        assert_eq!(current.version, "1.0.0");
        assert_eq!(legacy.version, "0.9.0");
        assert_ne!(current.adapter_digest, legacy.adapter_digest);
    }

    #[test]
    fn configured_process_run_persists_attempt_then_trusted_receipt() {
        let conn = conn();
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".planr")).unwrap();
        fs::write(
            root.path().join(".planr/evidence.yaml"),
            repository_policy_yaml(),
        )
        .unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract =
            execution_contract("sh", vec!["-c", "printf '{\"contains\":\"ready\"}'"], 5000);
        seed(&conn, &obligation, &instance, &contract);

        let cancellation = CancellationToken::new();
        let input = run_input(root.path(), obligation, instance, contract, &cancellation);
        let expected_policy_digest =
            super::super::policy::load_repository_policy_binding(root.path())
                .unwrap()
                .unwrap()
                .digest
                .as_str()
                .to_string();
        let output = run_configured_process_adapter(&conn, input).unwrap();
        let expected_config_digest = output.receipt_value["config_digest"]
            .as_str()
            .unwrap()
            .to_string();

        assert_eq!(output.attempt.status, AttemptStatus::Passed);
        assert_eq!(
            output.receipt_value["receipt_status"],
            json!(ReceiptStatus::Trusted.as_str())
        );
        assert_eq!(
            output.receipt_value["provenance"]["assigned_by"],
            json!("planr")
        );
        assert_eq!(
            output.receipt_value["target"],
            json!({"kind": "process", "uri": "local://process"})
        );
        assert_eq!(
            output.receipt_value["fixture_disclosure"]["fixtures_used"],
            json!(true)
        );
        assert_eq!(
            output.receipt_value["config_digest"],
            json!(expected_config_digest)
        );
        let receipt_attempt: String = conn
            .query_row(
                "SELECT attempt_id FROM evidence_receipts WHERE id = ?1",
                [output.receipt_value["id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(receipt_attempt, output.attempt.id.as_str());
        let trusted_binding_json: String = conn
            .query_row(
                "SELECT trusted_binding_json FROM evidence_receipts WHERE id = ?1",
                [output.receipt_value["id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let trusted_binding: Value = serde_json::from_str(&trusted_binding_json).unwrap();
        assert_eq!(trusted_binding["policy_digest"], expected_policy_digest);
        assert_eq!(trusted_binding["policy_source"], "repository");
        assert_eq!(trusted_binding["source"], output.receipt_value["source"]);
        assert!(
            conn.execute(
                "UPDATE evidence_attempts SET attempt_status = 'failed' WHERE id = ?1",
                [output.attempt.id.as_str()],
            )
            .is_err()
        );
    }

    #[test]
    fn configured_process_run_persists_empty_success_attempt_under_status_trigger() {
        let conn = conn();
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".planr")).unwrap();
        fs::write(
            root.path().join(".planr/evidence.yaml"),
            repository_policy_yaml(),
        )
        .unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract =
            execution_contract("sh", vec!["-c", "printf '{\"contains\":\"ready\"}'"], 5000);
        seed(&conn, &obligation, &instance, &contract);

        let cancellation = CancellationToken::new();
        let output = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap();

        assert_eq!(output.attempt.status, AttemptStatus::Passed);
        assert_eq!(output.attempt.exit["exit_code"], json!(0));
        assert_eq!(output.attempt.exit["error"], Value::Null);
        let attempt_json: Value = conn
            .query_row(
                "SELECT attempt_json FROM evidence_attempts WHERE id = ?1",
                [output.attempt.id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .map(|value| serde_json::from_str(&value).unwrap())
            .unwrap();
        assert_eq!(attempt_json["status"], json!("passed"));
        assert_eq!(attempt_json["exit"]["exit_code"], json!(0));
        assert_eq!(attempt_json["exit"]["error"], Value::Null);
    }

    #[test]
    fn configured_process_run_rejects_ambient_transaction_before_launch_or_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "ambient-transaction.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        conn.execute_batch("BEGIN").unwrap();

        let error = run_configured_process_adapter(
            &conn,
            run_input(
                root.path(),
                obligation,
                instance,
                contract,
                &CancellationToken::new(),
            ),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("autocommit SQLite connection"), "{error}");
        assert_no_attempts_or_receipts(&conn);
        assert_marker_absent(root.path(), marker);
        conn.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn configured_process_run_records_failed_attempt_without_receipt_when_tracked_source_changes() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = shell_contract(
            "printf changed > .planr/repository-snapshot-anchor; printf ok".to_string(),
        );
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(
                "repository evidence source no longer matches launch snapshot before trusted receipt acceptance"
            ),
            "{error}"
        );
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_latest_attempt_failed_with_gap(&conn, "stale_source");
    }

    #[test]
    fn configured_process_run_records_failed_attempt_without_receipt_when_untracked_content_changes()
     {
        let conn = conn();
        let root = tempdir().unwrap();
        fs::write(root.path().join("same-untracked.txt"), "before\n").unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = shell_contract("printf after > same-untracked.txt; printf ok".to_string());
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(
                "repository evidence source no longer matches launch snapshot before trusted receipt acceptance"
            ),
            "{error}"
        );
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_latest_attempt_failed_with_gap(&conn, "stale_source");
    }

    #[test]
    fn configured_process_run_records_failed_attempt_without_receipt_when_policy_changes() {
        let conn = conn();
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".planr")).unwrap();
        let original_policy = repository_policy_yaml();
        let changed_policy = repository_policy_yaml_with_max_age_seconds(7200);
        fs::write(root.path().join(".planr/evidence.yaml"), &original_policy).unwrap();
        let original_digest = super::super::policy::load_repository_policy_binding(root.path())
            .unwrap()
            .unwrap()
            .digest
            .as_str()
            .to_string();
        fs::write(root.path().join(".planr/evidence.yaml"), &changed_policy).unwrap();
        let changed_digest = super::super::policy::load_repository_policy_binding(root.path())
            .unwrap()
            .unwrap()
            .digest
            .as_str()
            .to_string();
        assert_ne!(changed_digest, original_digest);
        fs::write(root.path().join(".planr/evidence.yaml"), original_policy).unwrap();
        fs::write(root.path().join(".planr/evidence-p2.yaml"), changed_policy).unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = shell_contract(
            "cp .planr/evidence-p2.yaml .planr/evidence.yaml; printf ok".to_string(),
        );
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(
                "repository evidence policy no longer matches launch snapshot before trusted receipt acceptance"
            ),
            "{error}"
        );
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_latest_attempt_failed_with_gap(&conn, "stale_policy");
    }

    #[test]
    fn configured_process_run_records_failed_attempt_without_receipt_when_policy_is_removed() {
        let conn = conn();
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".planr")).unwrap();
        fs::write(
            root.path().join(".planr/evidence.yaml"),
            repository_policy_yaml(),
        )
        .unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = shell_contract("rm .planr/evidence.yaml; printf ok".to_string());
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains(
                "repository evidence policy no longer matches launch snapshot before trusted receipt acceptance"
            ),
            "{error}"
        );
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_latest_attempt_failed_with_gap(&conn, "stale_policy");
    }

    #[test]
    fn configured_process_run_attempt_validates_against_frozen_schema() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = execution_contract("sh", vec!["-c", "printf $PLANR_TEST_VALUE"], 5000);
        seed(&conn, &obligation, &instance, &contract);

        let output = run_configured_process_adapter(
            &conn,
            run_input(
                root.path(),
                obligation,
                instance,
                contract,
                &CancellationToken::new(),
            ),
        )
        .unwrap();
        let schema: Value = serde_json::from_str(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        let attempt_value = serde_json::to_value(&output.attempt).unwrap();
        let errors = validator
            .iter_errors(&attempt_value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();

        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn configured_process_run_records_fail_timeout_cancel_and_output_bounds() {
        let conn = conn();
        let root = tempdir().unwrap();
        for (idx, (args, timeout, cancel, expected)) in [
            (vec!["-c", "exit 2"], 5000, false, AttemptStatus::Failed),
            (vec!["-c", "sleep 1"], 20, false, AttemptStatus::TimedOut),
            (
                vec!["-c", "printf too-long"],
                5000,
                false,
                AttemptStatus::Unavailable,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut obligation = obligation();
            obligation.id = EvidenceId::parse(format!("obl-process-{idx}")).unwrap();
            let mut instance = instance();
            instance.id = EvidenceId::parse(format!("capinst-process-{idx}")).unwrap();
            let mut contract = execution_contract("sh", args, timeout);
            if expected == AttemptStatus::Unavailable {
                contract.stdout_limit_bytes = 1;
            }
            seed(&conn, &obligation, &instance, &contract);
            let cancellation = CancellationToken::new();
            if cancel {
                cancellation.cancel();
            }
            let output = run_configured_process_adapter(
                &conn,
                run_input(root.path(), obligation, instance, contract, &cancellation),
            )
            .unwrap();
            assert_eq!(output.attempt.status, expected);
            if expected == AttemptStatus::Unavailable {
                assert_ne!(
                    output.attempt.stdout_digest.as_str(),
                    sha256_prefixed_bytes(&[])
                );
                assert_eq!(output.attempt.raw_result["stdout_excerpt"], json!("t"));
            }
        }

        let mut obligation = obligation();
        obligation.id = EvidenceId::parse("obl-process-cancel").unwrap();
        let mut instance = instance();
        instance.id = EvidenceId::parse("capinst-process-cancel").unwrap();
        let contract = execution_contract("sh", vec!["-c", "sleep 1"], 5000);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let output = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap();
        assert_eq!(output.attempt.status, AttemptStatus::Aborted);
    }

    #[test]
    fn configured_process_run_maps_boundary_gaps_from_structured_signal_not_exit_code() {
        let conn = conn();
        let root = tempdir().unwrap();
        for (idx, script, expected_gap) in [
            (
                0,
                "printf '{\"planr_adapter_boundary\":\"sandbox_blocked\"}\\n' >&2; exit 2",
                GapReason::SandboxBlocked,
            ),
            (1, "exit 77", GapReason::ProductFailed),
        ] {
            let mut obligation = obligation();
            obligation.id = EvidenceId::parse(format!("obl-boundary-{idx}")).unwrap();
            let mut instance = instance();
            instance.id = EvidenceId::parse(format!("capinst-boundary-{idx}")).unwrap();
            let contract = execution_contract("sh", vec!["-c", script], 5000);
            seed(&conn, &obligation, &instance, &contract);
            let output = run_configured_process_adapter(
                &conn,
                run_input(
                    root.path(),
                    obligation,
                    instance,
                    contract,
                    &CancellationToken::new(),
                ),
            )
            .unwrap();

            assert_eq!(output.attempt.status, AttemptStatus::Failed);
            assert_eq!(
                output.receipt_value["proof_gaps"],
                json!([expected_gap.as_str()])
            );
        }
    }

    #[test]
    fn configured_process_run_persists_observed_output_bounds_per_stream() {
        let conn = conn();
        let root = tempdir().unwrap();
        for (idx, args, stdout_limit, stderr_limit, expected_status, expected_bounds) in [
            (
                0,
                vec!["-c", "printf '{\"contains\":\"ready\"}'; printf err >&2"],
                1024,
                1024,
                AttemptStatus::Passed,
                json!({
                    "stdout_bytes": 20,
                    "stderr_bytes": 3,
                    "stdout_truncated": false,
                    "stderr_truncated": false,
                }),
            ),
            (
                1,
                vec!["-c", "printf err >&2; printf abcdef"],
                3,
                10,
                AttemptStatus::Unavailable,
                json!({
                    "stdout_bytes": 3,
                    "stderr_bytes": 3,
                    "stdout_truncated": true,
                    "stderr_truncated": false,
                }),
            ),
            (
                2,
                vec!["-c", "printf out; printf abcdef >&2"],
                10,
                3,
                AttemptStatus::Unavailable,
                json!({
                    "stdout_bytes": 3,
                    "stderr_bytes": 3,
                    "stdout_truncated": false,
                    "stderr_truncated": true,
                }),
            ),
        ] {
            let mut obligation = obligation();
            obligation.id = EvidenceId::parse(format!("obl-process-bounds-{idx}")).unwrap();
            let mut instance = instance();
            instance.id = EvidenceId::parse(format!("capinst-process-bounds-{idx}")).unwrap();
            let mut contract = execution_contract("sh", args, 5000);
            contract.stdout_limit_bytes = stdout_limit;
            contract.stderr_limit_bytes = stderr_limit;
            let expected_sandbox_limits = json!({
                "timeout_ms": contract.timeout_ms,
                "stdout_bytes": contract.stdout_limit_bytes,
                "stderr_bytes": contract.stderr_limit_bytes,
            });
            seed(&conn, &obligation, &instance, &contract);

            let output = run_configured_process_adapter(
                &conn,
                run_input(
                    root.path(),
                    obligation,
                    instance,
                    contract,
                    &CancellationToken::new(),
                ),
            )
            .unwrap();
            assert_eq!(output.attempt.status, expected_status);
            assert_eq!(output.attempt.output_bounds, expected_bounds);
            assert_raw_result_has_only_observed_output_facts(&output.attempt.raw_result);
            assert_eq!(
                output.receipt_value["sandbox"]["limits"],
                expected_sandbox_limits
            );
            let persisted_bounds: String = conn
                .query_row(
                    "SELECT json_extract(attempt_json, '$.output_bounds') FROM evidence_attempts WHERE id = ?1",
                    [output.attempt.id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&persisted_bounds).unwrap(),
                expected_bounds
            );
        }
    }

    #[test]
    fn configured_process_raw_result_error_paths_do_not_embed_configured_limits() {
        let root = tempdir().unwrap();
        let resolved = ResolvedProcessRun {
            cwd: root.path().to_path_buf(),
            argv: vec!["/bin/sh".to_string(), "-c".to_string(), "true".to_string()],
            env: BTreeMap::new(),
            command_identity: json!({
                "kind": "command",
                "command": ["/bin/sh", "-c", "true"],
                "cwd": root.path().to_string_lossy()
            }),
        };

        let generic = attempt_result(Err(anyhow::anyhow!("spawn_failed")), &resolved).unwrap();
        assert_eq!(generic.output_bounds["stdout_bytes"], json!(0));
        assert_eq!(generic.output_bounds["stderr_bytes"], json!(0));
        assert_raw_result_has_only_observed_output_facts(&generic.raw_result);

        let output_limit = BoundedProcessOutput {
            argv: resolved.argv.clone(),
            exit_code: None,
            timed_out: false,
            interrupted: false,
            output_limit_exceeded: true,
            stdout_digest: DIGEST_A.to_string(),
            stderr_digest: DIGEST_B.to_string(),
            stdout_excerpt: "abc".to_string(),
            stderr_excerpt: "err".to_string(),
            stdout_bytes: 3,
            stderr_bytes: 3,
            stdout_truncated: true,
            stderr_truncated: false,
            #[cfg(test)]
            process_tree_term_grace_sleeps: 0,
        };
        let limit = process_error_result(output_limit, &resolved).unwrap();
        assert_eq!(limit.output_bounds["stdout_bytes"], json!(3));
        assert_eq!(limit.output_bounds["stderr_truncated"], json!(false));
        assert_raw_result_has_only_observed_output_facts(&limit.raw_result);
    }

    #[test]
    fn configured_process_run_rejects_unsafe_resolution_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = execution_contract("/bin/sh", vec!["-c", "true"], 5000);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("bare command"), "{error}");
        let attempts: i64 = conn
            .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(attempts, 0);
    }

    #[test]
    fn configured_process_run_records_three_attempt_retry_lineage() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "ordered-retry.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        let first = run_configured_process_adapter(
            &conn,
            run_input(
                root.path(),
                obligation.clone(),
                instance.clone(),
                contract.clone(),
                &cancellation,
            ),
        )
        .unwrap();
        let mut second_input = run_input(
            root.path(),
            obligation.clone(),
            instance.clone(),
            contract.clone(),
            &cancellation,
        );
        second_input.retry_of = Some(first.attempt.id.clone());
        second_input.attempt_index = 1;
        let second = run_configured_process_adapter(&conn, second_input).unwrap();
        let mut third_input = run_input(root.path(), obligation, instance, contract, &cancellation);
        third_input.retry_of = Some(second.attempt.id.clone());
        third_input.attempt_index = 2;
        let third = run_configured_process_adapter(&conn, third_input).unwrap();
        let previous_attempt_ids: String = conn
            .query_row(
                "SELECT json_extract(attempt_json, '$.retry_lineage.previous_attempt_ids') FROM evidence_attempts WHERE id = ?1",
                [third.attempt.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            previous_attempt_ids,
            json!([first.attempt.id.as_str(), second.attempt.id.as_str()]).to_string()
        );
        assert_eq!(third.attempt.retry_lineage["attempt_number"], json!(3));
        assert_eq!(third.attempt.retry_lineage["max_attempts"], json!(3));
        let persisted_predecessor: String = conn
            .query_row(
                "SELECT retry_predecessor_attempt_id FROM evidence_attempts WHERE id = ?1",
                [third.attempt.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted_predecessor, second.attempt.id.as_str());
        let second_receipt_id = second.receipt_value["id"].as_str().unwrap();
        let third_supersedes: String = conn
            .query_row(
                "SELECT supersedes_receipt_id FROM evidence_receipts WHERE attempt_id = ?1",
                [third.attempt.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(third_supersedes, second_receipt_id);
        let first_receipt_id = first.receipt_value["id"].as_str().unwrap();
        let second_supersedes: String = conn
            .query_row(
                "SELECT supersedes_receipt_id FROM evidence_receipts WHERE attempt_id = ?1",
                [second.attempt.id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(second_supersedes, first_receipt_id);
    }

    #[test]
    fn configured_process_run_rejects_retry_after_max_attempts_before_launch() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "cap-exhausted.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        let mut first_input = run_input(
            root.path(),
            obligation.clone(),
            instance.clone(),
            contract.clone(),
            &cancellation,
        );
        first_input.max_attempts = 1;
        let first = run_configured_process_adapter(&conn, first_input).unwrap();
        fs::remove_file(root.path().join(marker)).unwrap();
        let mut retry_input = run_input(root.path(), obligation, instance, contract, &cancellation);
        retry_input.retry_of = Some(first.attempt.id.clone());
        retry_input.attempt_index = 1;
        retry_input.max_attempts = 1;

        let error = run_configured_process_adapter(&conn, retry_input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("max_attempts exhausted"), "{error}");
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 1);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_missing_retry_predecessor_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "missing-predecessor.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.retry_of = Some(EvidenceId::parse("eatt-missing").unwrap());
        input.attempt_index = 1;

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("retry predecessor"), "{error}");
        assert_no_attempts_or_receipts(&conn);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_invalid_initial_retry_before_launch() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "invalid-initial.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.attempt_index = 1;

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("initial evidence attempt"), "{error}");
        assert_no_attempts_or_receipts(&conn);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_cross_project_retry_before_launch() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "cross-project.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        insert_prior_attempt(
            &conn,
            "eatt-other-project",
            "p-other",
            obligation.id.as_str(),
            instance.id.as_str(),
            0,
            None,
        );
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.retry_of = Some(EvidenceId::parse("eatt-other-project").unwrap());
        input.attempt_index = 1;

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("different project"), "{error}");
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_cross_obligation_retry_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let mut other_obligation = obligation.clone();
        other_obligation.id = EvidenceId::parse("obl-process-other").unwrap();
        let instance = instance();
        let marker = "cross-obligation.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        seed_obligation_only(&conn, &other_obligation);
        insert_prior_attempt(
            &conn,
            "eatt-other-obligation",
            "p-evidence",
            other_obligation.id.as_str(),
            instance.id.as_str(),
            0,
            None,
        );
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.retry_of = Some(EvidenceId::parse("eatt-other-obligation").unwrap());
        input.attempt_index = 1;

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("different obligation"), "{error}");
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_cross_capability_retry_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let mut other_instance = instance.clone();
        other_instance.id = EvidenceId::parse("capinst-process-other").unwrap();
        let marker = "cross-capability.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        seed_instance_only(&conn, &other_instance, &contract);
        insert_prior_attempt(
            &conn,
            "eatt-other-capability",
            "p-evidence",
            obligation.id.as_str(),
            other_instance.id.as_str(),
            0,
            None,
        );
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.retry_of = Some(EvidenceId::parse("eatt-other-capability").unwrap());
        input.attempt_index = 1;

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("different capability"), "{error}");
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_non_contiguous_retry_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let marker = "non-contiguous.marker";
        let contract = marker_contract(marker);
        seed(&conn, &obligation, &instance, &contract);
        insert_prior_attempt(
            &conn,
            "eatt-prior-zero",
            "p-evidence",
            obligation.id.as_str(),
            instance.id.as_str(),
            0,
            None,
        );
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.retry_of = Some(EvidenceId::parse("eatt-prior-zero").unwrap());
        input.attempt_index = 2;

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("attempt_index"), "{error}");
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 0);
        assert_marker_absent(root.path(), marker);
    }

    #[test]
    fn configured_process_run_rejects_contract_mismatch_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let cancellation = CancellationToken::new();
        let canonical = execution_contract("sh", vec!["-c", "true"], 5000);
        seed(&conn, &obligation, &instance, &canonical);
        let mut contract = canonical;
        contract.payload_schema.schema_ref = "example.process.other@v1".to_string();

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("registered adapter contract"), "{error}");
        assert_no_attempts_or_receipts(&conn);
    }

    #[test]
    fn configured_process_run_rejects_full_execution_contract_drift_before_persistence() {
        let cases: [(&str, ContractMutation); 7] = [
            ("executable", |contract: &mut ProcessExecutionContract| {
                contract.executable = "printf".to_string();
            }),
            ("arguments", |contract: &mut ProcessExecutionContract| {
                contract.args = vec!["-c".to_string(), "exit 0".to_string()];
            }),
            (
                "working_directory",
                |contract: &mut ProcessExecutionContract| {
                    contract.working_directory = Some("src".to_string());
                },
            ),
            (
                "timeout_limit",
                |contract: &mut ProcessExecutionContract| {
                    contract.timeout_ms += 1;
                },
            ),
            ("stdout_limit", |contract: &mut ProcessExecutionContract| {
                contract.stdout_limit_bytes += 1;
            }),
            ("stderr_limit", |contract: &mut ProcessExecutionContract| {
                contract.stderr_limit_bytes += 1;
            }),
            (
                "schema_digest",
                |contract: &mut ProcessExecutionContract| {
                    contract.payload_schema.schema_digest = Sha256Digest::parse(DIGEST_A).unwrap();
                },
            ),
        ];
        for (idx, (name, mutate)) in cases.into_iter().enumerate() {
            let conn = conn();
            let root = tempdir().unwrap();
            let mut obligation = obligation();
            obligation.id = EvidenceId::parse(format!("obl-process-drift-{idx}")).unwrap();
            let mut instance = instance();
            instance.id = EvidenceId::parse(format!("capinst-process-drift-{idx}")).unwrap();
            let canonical = execution_contract("sh", vec!["-c", "true"], 5000);
            seed(&conn, &obligation, &instance, &canonical);
            let mut supplied = canonical.clone();
            mutate(&mut supplied);
            let cancellation = CancellationToken::new();

            let error = run_configured_process_adapter(
                &conn,
                run_input(root.path(), obligation, instance, supplied, &cancellation),
            )
            .unwrap_err()
            .to_string();

            assert!(
                error.contains("registered adapter contract"),
                "{name}: {error}"
            );
            assert_no_attempts_or_receipts(&conn);
        }
    }

    #[test]
    fn configured_process_run_rejects_unsupported_observation_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let mut obligation = obligation();
        obligation.observations[0].observation_type =
            super::super::NamespacedIdentifier::parse("example.process.other").unwrap();
        let instance = instance();
        let cancellation = CancellationToken::new();
        let contract = execution_contract("sh", vec!["-c", "true"], 5000);
        seed(&conn, &obligation, &instance, &contract);

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("observation type"), "{error}");
        assert_no_attempts_or_receipts(&conn);
    }

    #[test]
    fn configured_process_run_rejects_runtime_binding_mismatch_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = execution_contract("sh", vec!["-c", "true"], 5000);
        seed(&conn, &obligation, &instance, &contract);
        let cancellation = CancellationToken::new();
        let mut input = run_input(root.path(), obligation, instance, contract, &cancellation);
        input.environment.id = EvidenceId::parse("other-shell").unwrap();

        let error = run_configured_process_adapter(&conn, input)
            .unwrap_err()
            .to_string();

        assert!(error.contains("environment"), "{error}");
        assert_no_attempts_or_receipts(&conn);
    }

    #[test]
    fn configured_process_run_rejects_fixture_policy_mismatch_before_persistence() {
        let conn = conn();
        let root = tempdir().unwrap();
        let mut obligation = obligation();
        obligation.fixture_policy["fixtures_allowed"] = json!(false);
        let instance = instance();
        let cancellation = CancellationToken::new();
        let contract = execution_contract("sh", vec!["-c", "true"], 5000);
        seed(&conn, &obligation, &instance, &contract);

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("fixtures disallowed"), "{error}");
        assert_no_attempts_or_receipts(&conn);
    }

    #[test]
    fn configured_process_run_rolls_back_attempt_when_receipt_insert_fails() {
        let conn = conn();
        let root = tempdir().unwrap();
        let obligation = obligation();
        let instance = instance();
        let contract = execution_contract("sh", vec!["-c", "true"], 5000);
        seed(&conn, &obligation, &instance, &contract);
        conn.execute("DROP TABLE evidence_receipts", []).unwrap();
        assert!(conn.is_autocommit());
        let cancellation = CancellationToken::new();

        let error = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("evidence_receipts"), "{error}");
        let attempts: i64 = conn
            .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(attempts, 0);
    }

    #[test]
    fn strict_structured_observation_results_rejects_ambiguous_or_mismatched_payloads() {
        let mut obligation = obligation();
        obligation.observations = serde_json::from_value(json!([
            {
                "id": "obs-visible",
                "type": "example.process.stdout",
                "subject": "visible",
                "expected": {"visible": true},
                "target": {"kind": "process", "uri": "local://process"}
            },
            {
                "id": "obs-console",
                "type": "example.process.stdout",
                "subject": "console",
                "expected": {"error_count": 0},
                "target": {"kind": "process", "uri": "local://process"}
            }
        ]))
        .unwrap();
        let mut execution = execution_contract("sh", vec!["-c", "true"], 5000);
        execution.payload_schema.schema_ref =
            "schema://planr.structured_observation_results.v1".to_string();
        let target: TargetBinding =
            serde_json::from_value(json!({"kind": "process", "uri": "local://process"})).unwrap();
        let environment: EnvironmentBinding =
            serde_json::from_value(json!({"kind": "local", "id": "dev-shell", "digest": DIGEST_B}))
                .unwrap();
        let fixture_disclosure = FixtureDisclosure {
            fixtures_used: false,
            mocks_used: false,
            fixture_refs: None,
            mock_refs: None,
        };
        let repository_root = Path::new(".");
        let digest = sha256_json_digest(&serde_json::to_value(&execution).unwrap()).unwrap();
        let valid_observations = json!([
            {
                "requirement_id": "obs-visible",
                "type": "example.process.stdout",
                "actual": {
                    "schema_ref": "schema://planr.structured_observation_results.v1",
                    "visible": true
                }
            },
            {
                "requirement_id": "obs-console",
                "type": "example.process.stdout",
                "actual": {
                    "schema_ref": "schema://planr.structured_observation_results.v1",
                    "error_count": 0
                }
            }
        ]);
        let payload = |observations: Value| {
            json!({
                "schema_version": "planr.structured_observation_results.v1",
                "target": serde_json::to_value(&target).unwrap(),
                "observed_target": {
                    "kind": "process",
                    "initial_uri": "local://process",
                    "final_uri": "local://process"
                },
                "environment": serde_json::to_value(&environment).unwrap(),
                "execution_contract_digest": digest,
                "fixture_disclosure": serde_json::to_value(&fixture_disclosure).unwrap(),
                "observations": observations,
            })
        };
        assert!(
            strict_structured_observation_results(
                &obligation,
                &execution,
                &BTreeMap::new(),
                StructuredObservationContext {
                    target: &target,
                    environment: &environment,
                    fixture_disclosure: &fixture_disclosure,
                    repository_root,
                },
                &raw_result_for_stdout(&payload(valid_observations.clone()).to_string(), false),
            )
            .is_ok()
        );
        for (label, stdout) in [
            (
                "missing id",
                payload(json!([valid_observations[0].clone()])).to_string(),
            ),
            (
                "duplicate id",
                payload(json!([
                    valid_observations[0].clone(),
                    valid_observations[0].clone()
                ]))
                .to_string(),
            ),
            (
                "wrong type",
                payload(json!([
                    {
                        "requirement_id": "obs-visible",
                        "type": "example.process.other",
                        "actual": {
                            "schema_ref": "schema://planr.structured_observation_results.v1",
                            "visible": true
                        }
                    },
                    valid_observations[1].clone()
                ]))
                .to_string(),
            ),
            (
                "wrong schema",
                payload(json!([
                    {
                        "requirement_id": "obs-visible",
                        "type": "example.process.stdout",
                        "actual": {"schema_ref": "schema://wrong", "visible": true}
                    },
                    valid_observations[1].clone()
                ]))
                .to_string(),
            ),
            (
                "expected mismatch",
                payload(json!([
                    {
                        "requirement_id": "obs-visible",
                        "type": "example.process.stdout",
                        "actual": {
                            "schema_ref": "schema://planr.structured_observation_results.v1",
                            "visible": false
                        }
                    },
                    valid_observations[1].clone()
                ]))
                .to_string(),
            ),
            ("malformed", "{".to_string()),
            (
                "log-prefixed",
                format!("log line\n{}", payload(valid_observations.clone())),
            ),
            (
                "extra unknown id",
                payload(json!([
                    valid_observations[0].clone(),
                    valid_observations[1].clone(),
                    {
                        "requirement_id": "obs-extra",
                        "type": "example.process.stdout",
                        "actual": {
                            "schema_ref": "schema://planr.structured_observation_results.v1"
                        }
                    }
                ]))
                .to_string(),
            ),
            ("wrong target", {
                let mut value = payload(valid_observations.clone());
                value["target"] = json!({"kind": "process", "uri": "local://other"});
                value.to_string()
            }),
            ("missing observed target", {
                let mut value = payload(valid_observations.clone());
                value.as_object_mut().unwrap().remove("observed_target");
                value.to_string()
            }),
            ("wrong observed target", {
                let mut value = payload(valid_observations.clone());
                value["observed_target"]["initial_uri"] = json!("local://hard-coded");
                value.to_string()
            }),
            ("wrong environment", {
                let mut value = payload(valid_observations.clone());
                value["environment"]["id"] = json!("other-shell");
                value.to_string()
            }),
            ("wrong execution digest", {
                let mut value = payload(valid_observations.clone());
                value["execution_contract_digest"] = json!(DIGEST_A);
                value.to_string()
            }),
        ] {
            assert!(
                strict_structured_observation_results(
                    &obligation,
                    &execution,
                    &BTreeMap::new(),
                    StructuredObservationContext {
                        target: &target,
                        environment: &environment,
                        fixture_disclosure: &fixture_disclosure,
                        repository_root,
                    },
                    &raw_result_for_stdout(&stdout, false),
                )
                .is_err(),
                "{label} should fail closed"
            );
        }
        assert!(
            strict_structured_observation_results(
                &obligation,
                &execution,
                &BTreeMap::new(),
                StructuredObservationContext {
                    target: &target,
                    environment: &environment,
                    fixture_disclosure: &fixture_disclosure,
                    repository_root,
                },
                &raw_result_for_stdout(&payload(valid_observations).to_string(), true),
            )
            .unwrap_err()
            .to_string()
            .contains("truncated")
        );
    }

    #[test]
    fn strict_structured_observation_results_validates_extracted_payload_schema() {
        let mut obligation = obligation();
        obligation.observations = serde_json::from_value(json!([{
            "id": "obs-visible",
            "type": "example.process.stdout",
            "subject": "visible",
            "expected": {"visible": true},
            "target": {"kind": "process", "uri": "local://process"},
            "payload_schema": {"schema_ref": "example.visible.result@v1"}
        }]))
        .unwrap();
        let mut execution = execution_contract("sh", vec!["-c", "true"], 5000);
        execution.payload_schema.schema_ref =
            "schema://planr.structured_observation_results.v1".to_string();
        let target: TargetBinding =
            serde_json::from_value(json!({"kind": "process", "uri": "local://process"})).unwrap();
        let environment: EnvironmentBinding =
            serde_json::from_value(json!({"kind": "local", "id": "dev-shell", "digest": DIGEST_B}))
                .unwrap();
        let fixture_disclosure = FixtureDisclosure {
            fixtures_used: false,
            mocks_used: false,
            fixture_refs: None,
            mock_refs: None,
        };
        let digest = sha256_json_digest(&serde_json::to_value(&execution).unwrap()).unwrap();
        let payload = |visible: Value| {
            json!({
                "schema_version": "planr.structured_observation_results.v1",
                "target": serde_json::to_value(&target).unwrap(),
                "observed_target": {
                    "kind": "process",
                    "initial_uri": "local://process",
                    "final_uri": "local://process"
                },
                "environment": serde_json::to_value(&environment).unwrap(),
                "execution_contract_digest": digest,
                "fixture_disclosure": serde_json::to_value(&fixture_disclosure).unwrap(),
                "observations": [{
                    "requirement_id": "obs-visible",
                    "type": "example.process.stdout",
                    "actual": {
                        "schema_ref": "example.visible.result@v1",
                        "visible": visible
                    }
                }]
            })
        };
        let schemas = BTreeMap::from([(
            "obs-visible".to_string(),
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["schema_ref", "visible"],
                "properties": {
                    "schema_ref": {"const": "example.visible.result@v1"},
                    "visible": {"type": "boolean"}
                }
            }),
        )]);

        let valid = strict_structured_observation_results(
            &obligation,
            &execution,
            &schemas,
            StructuredObservationContext {
                target: &target,
                environment: &environment,
                fixture_disclosure: &fixture_disclosure,
                repository_root: Path::new("."),
            },
            &raw_result_for_stdout(&payload(json!(true)).to_string(), false),
        )
        .unwrap();
        assert_eq!(valid["obs-visible"]["visible"], true);

        let error = strict_structured_observation_results(
            &obligation,
            &execution,
            &schemas,
            StructuredObservationContext {
                target: &target,
                environment: &environment,
                fixture_disclosure: &fixture_disclosure,
                repository_root: Path::new("."),
            },
            &raw_result_for_stdout(&payload(json!("yes")).to_string(), false),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("payload schema mismatch for obs-visible"),
            "{error}"
        );
        assert!(error.contains("/visible"), "{error}");
    }

    #[test]
    fn configured_process_run_persists_structured_output_failure_as_untrusted_attempt() {
        let conn = conn();
        let root = tempdir().unwrap();
        let mut obligation = obligation();
        obligation.observations[0].expected = json!({"visible": true});
        let mut instance = instance();
        instance.observed_payload_contract.schema_ref =
            "schema://planr.structured_observation_results.v1".to_string();
        let cancellation = CancellationToken::new();
        let mut contract = execution_contract("sh", vec!["-c", "printf 'not json'"], 5000);
        contract.payload_schema.schema_ref =
            "schema://planr.structured_observation_results.v1".to_string();
        seed(&conn, &obligation, &instance, &contract);

        let output = run_configured_process_adapter(
            &conn,
            run_input(root.path(), obligation, instance, contract, &cancellation),
        )
        .unwrap();

        assert_eq!(output.attempt.status, AttemptStatus::Failed);
        assert_eq!(output.attempt.exit["error"], "verifier_failed");
        assert_eq!(
            output.attempt.raw_result["planr_adapter_gap_reasons"],
            json!(["verifier_failed"])
        );
        assert!(
            output.attempt.raw_result["structured_observation_error"]
                .as_str()
                .unwrap()
                .contains("single JSON"),
            "{}",
            output.attempt.raw_result
        );
        assert_eq!(
            output.receipt_value["proof_gaps"],
            json!(["verifier_failed"])
        );
        assert_eq!(output.receipt_value["observations"][0]["outcome"], "failed");
        assert_attempt_count(&conn, 1);
        assert_receipt_count(&conn, 1);
    }

    fn raw_result_for_stdout(stdout: &str, truncated: bool) -> Value {
        json!({
            "kind": "process_output",
            "stdout_excerpt": stdout,
            "stderr_excerpt": "",
            "stdout_bytes": stdout.len(),
            "stderr_bytes": 0,
            "stdout_truncated": truncated,
            "stderr_truncated": false,
            "stdout_digest": sha256_prefixed_bytes(stdout.as_bytes()),
            "stderr_digest": sha256_prefixed_bytes(b""),
        })
    }

    fn assert_no_attempts_or_receipts(conn: &Connection) {
        assert_attempt_count(conn, 0);
        assert_receipt_count(conn, 0);
    }

    fn assert_attempt_count(conn: &Connection, expected: i64) {
        let attempts: i64 = conn
            .query_row("SELECT COUNT(*) FROM evidence_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(attempts, expected);
    }

    fn assert_receipt_count(conn: &Connection, expected: i64) {
        let receipts: i64 = conn
            .query_row("SELECT COUNT(*) FROM evidence_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(receipts, expected);
    }

    fn assert_latest_attempt_failed_with_gap(conn: &Connection, expected_gap: &str) {
        let attempt_json: String = conn
            .query_row(
                "SELECT attempt_json FROM evidence_attempts ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let attempt: Value = serde_json::from_str(&attempt_json).unwrap();
        assert_eq!(attempt["status"], "failed");
        assert_eq!(attempt["exit"]["error"], expected_gap);
        assert_eq!(
            attempt["raw_result"]["planr_adapter_gap_reasons"],
            json!([expected_gap])
        );
        assert!(
            attempt["raw_result"]["repository_snapshot_error"]
                .as_str()
                .unwrap()
                .contains("before trusted receipt acceptance")
        );
    }

    fn assert_marker_absent(root: &Path, marker_name: &str) {
        assert!(
            !root.join(marker_name).exists(),
            "process marker {marker_name} must not be created before retry validation fails"
        );
    }

    fn assert_raw_result_has_only_observed_output_facts(raw_result: &Value) {
        assert!(raw_result.get("output_bounds").is_none(), "{raw_result}");
        assert!(raw_result.get("timeout_ms").is_none(), "{raw_result}");
        assert!(
            raw_result.get("stdout_limit_bytes").is_none(),
            "{raw_result}"
        );
        assert!(
            raw_result.get("stderr_limit_bytes").is_none(),
            "{raw_result}"
        );
        assert!(raw_result.get("stdout_bytes").is_some(), "{raw_result}");
        assert!(raw_result.get("stderr_bytes").is_some(), "{raw_result}");
        assert!(raw_result.get("stdout_truncated").is_some(), "{raw_result}");
        assert!(raw_result.get("stderr_truncated").is_some(), "{raw_result}");
    }
}
