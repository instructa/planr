use crate::canonical_json::sha256_json_digest;
use anyhow::{Result, bail};
use planr::evidence::{parse_untrusted_evidence_proposal, trusted_receipt_binding_matches_receipt};
use rusqlite::{Connection, OptionalExtension, functions::FunctionFlags};
use serde_json::{Map, Value};
use std::path::{Component, Path};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) fn ensure_evidence_schema(conn: &Connection) -> Result<()> {
    register_connection_functions(conn)?;
    if table_exists(conn, "proof_obligations")? {
        validate_existing_proof_obligation_containment(conn)?;
    }
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS proof_obligations(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  item_id TEXT,
  criterion_id TEXT NOT NULL,
  obligation_version INTEGER NOT NULL CHECK(obligation_version >= 1),
  title TEXT NOT NULL,
  binding INTEGER NOT NULL CHECK(binding IN (0, 1)),
  observation_requirements_json TEXT NOT NULL,
  fixture_policy_json TEXT NOT NULL DEFAULT '{}',
  freshness_policy_json TEXT NOT NULL DEFAULT '{}',
  assurance_policy_json TEXT NOT NULL DEFAULT '{}',
  policy_digest TEXT NOT NULL CHECK(policy_digest GLOB 'sha256:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
  config_digest TEXT NOT NULL CHECK(config_digest GLOB 'sha256:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
  source_digest TEXT,
  supersedes_obligation_id TEXT,
  created_at TEXT NOT NULL,
  retry_aggregation TEXT NOT NULL DEFAULT 'latest_applicable_pass'
    CHECK(retry_aggregation IN ('latest_applicable_pass','all_applicable_pass')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(item_id) REFERENCES items(id),
  FOREIGN KEY(supersedes_obligation_id) REFERENCES proof_obligations(id)
);
CREATE TABLE IF NOT EXISTS verification_capability_manifests(
  id TEXT NOT NULL,
  version TEXT NOT NULL,
  adapter_kind TEXT NOT NULL CHECK(adapter_kind IN ('process','host','mcp','artifact_import','user_attestation')),
  adapter_digest TEXT NOT NULL,
  manifest_digest TEXT NOT NULL UNIQUE,
  manifest_json TEXT NOT NULL,
  source_path TEXT,
  created_at TEXT NOT NULL,
  PRIMARY KEY(id, version)
);
CREATE TABLE IF NOT EXISTS verification_capability_instances(
  id TEXT PRIMARY KEY,
  manifest_id TEXT NOT NULL,
  manifest_version TEXT NOT NULL,
  manifest_digest TEXT NOT NULL,
  probe_execution_id TEXT NOT NULL,
  availability_status TEXT NOT NULL CHECK(availability_status IN ('available','unavailable','degraded','permission_denied','sandbox_blocked','unsupported','probe_failed')),
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
CREATE TABLE IF NOT EXISTS evidence_attempts(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  obligation_id TEXT NOT NULL,
  capability_instance_id TEXT NOT NULL,
  attempt_status TEXT NOT NULL CHECK(attempt_status IN ('passed','failed','skipped','timed_out','aborted','unavailable','inconclusive')),
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
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(obligation_id) REFERENCES proof_obligations(id),
  FOREIGN KEY(capability_instance_id) REFERENCES verification_capability_instances(id),
  FOREIGN KEY(retry_predecessor_attempt_id) REFERENCES evidence_attempts(id)
);
CREATE TABLE IF NOT EXISTS evidence_attempt_artifacts(
  id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL,
  artifact_id TEXT,
  logical_name TEXT NOT NULL,
  path TEXT,
  digest TEXT NOT NULL,
  size_bytes INTEGER CHECK(size_bytes IS NULL OR size_bytes >= 0),
  root_kind TEXT NOT NULL CHECK(root_kind IN ('project','planr','artifact_store','external')),
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  FOREIGN KEY(attempt_id) REFERENCES evidence_attempts(id),
  FOREIGN KEY(artifact_id) REFERENCES artifacts(id),
  CHECK(path IS NULL OR (
    path <> ''
    AND path <> '..'
    AND path NOT LIKE '/%'
    AND path NOT LIKE '../%'
    AND path NOT LIKE '%/../%'
    AND path NOT LIKE '%\%'
    AND path NOT GLOB '[A-Za-z]:*'
  )),
  UNIQUE(attempt_id, logical_name)
);
CREATE TABLE IF NOT EXISTS evidence_receipts(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  obligation_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  receipt_status TEXT NOT NULL CHECK(receipt_status IN ('trusted','rejected','untrusted','stale','superseded')),
  receipt_digest TEXT NOT NULL UNIQUE,
  trusted_binding_json TEXT NOT NULL,
  observations_json TEXT NOT NULL,
  provenance_json TEXT NOT NULL,
  receipt_json TEXT NOT NULL,
  supersedes_receipt_id TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(obligation_id) REFERENCES proof_obligations(id),
  FOREIGN KEY(attempt_id) REFERENCES evidence_attempts(id),
  FOREIGN KEY(supersedes_receipt_id) REFERENCES evidence_receipts(id),
  UNIQUE(attempt_id, receipt_digest)
);
CREATE TABLE IF NOT EXISTS evidence_validated_imports(
  project_id TEXT NOT NULL,
  id TEXT NOT NULL,
  digest TEXT NOT NULL CHECK(digest GLOB 'sha256:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
  proposal_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id),
  PRIMARY KEY(project_id, id)
);
CREATE TABLE IF NOT EXISTS evidence_observation_results(
  id TEXT PRIMARY KEY,
  receipt_id TEXT NOT NULL,
  observation_id TEXT NOT NULL,
  observation_type TEXT NOT NULL,
  subject TEXT NOT NULL,
  result_status TEXT NOT NULL CHECK(result_status IN ('covered','missing','unsatisfied','blocked','inconclusive','waived','stale')),
  payload_digest TEXT,
  payload_json TEXT NOT NULL,
  schema_digest TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(receipt_id) REFERENCES evidence_receipts(id),
  UNIQUE(receipt_id, observation_id)
);
CREATE TABLE IF NOT EXISTS coverage_verdicts(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL CHECK(scope_kind IN ('project','plan','item','criterion','obligation')),
  scope_id TEXT NOT NULL,
  binding_digest TEXT NOT NULL,
  policy_digest TEXT NOT NULL,
  config_digest TEXT NOT NULL,
  coverage_status TEXT NOT NULL CHECK(coverage_status IN ('satisfied','unsatisfied','blocked','inconclusive','waived','stale')),
  source_receipt_digest_set TEXT NOT NULL DEFAULT '[]',
  waiver_digest_set TEXT NOT NULL DEFAULT '[]',
  verdict_json TEXT NOT NULL,
  computed_at TEXT NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id),
  UNIQUE(project_id, scope_kind, scope_id, binding_digest, policy_digest, config_digest)
);
CREATE TABLE IF NOT EXISTS coverage_verdict_history(
  id TEXT PRIMARY KEY,
  verdict_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL,
  scope_id TEXT NOT NULL,
  binding_digest TEXT NOT NULL,
  policy_digest TEXT NOT NULL,
  config_digest TEXT NOT NULL,
  coverage_status TEXT NOT NULL,
  verdict_json TEXT NOT NULL,
  computed_at TEXT NOT NULL,
  FOREIGN KEY(verdict_id) REFERENCES coverage_verdicts(id),
  FOREIGN KEY(project_id) REFERENCES projects(id)
);
CREATE TABLE IF NOT EXISTS evidence_waivers(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  approval_item_id TEXT NOT NULL,
  obligation_id TEXT,
  observation_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL CHECK(scope_kind IN ('project','plan','item','criterion','goal','obligation','observation')),
  scope_id TEXT NOT NULL,
  waiver_digest TEXT NOT NULL UNIQUE,
  reason TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_by TEXT NOT NULL,
  waiver_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(approval_item_id) REFERENCES items(id),
  FOREIGN KEY(obligation_id) REFERENCES proof_obligations(id),
  CHECK(reason <> ''),
  CHECK(created_by <> '')
);
CREATE TABLE IF NOT EXISTS evidence_waiver_observations(
  waiver_id TEXT NOT NULL,
  observation_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  created_at TEXT NOT NULL,
  PRIMARY KEY(waiver_id, observation_id),
  FOREIGN KEY(waiver_id) REFERENCES evidence_waivers(id),
  CHECK(observation_id <> '')
);
CREATE INDEX IF NOT EXISTS proof_obligations_item_idx ON proof_obligations(item_id);
CREATE UNIQUE INDEX IF NOT EXISTS proof_obligations_project_identity_idx ON proof_obligations(id, project_id);
CREATE INDEX IF NOT EXISTS proof_obligations_policy_idx ON proof_obligations(project_id, policy_digest, config_digest);
CREATE INDEX IF NOT EXISTS verification_capability_instances_manifest_idx ON verification_capability_instances(manifest_id, manifest_version, availability_status);
CREATE UNIQUE INDEX IF NOT EXISTS evidence_attempts_project_obligation_identity_idx ON evidence_attempts(id, project_id, obligation_id);
CREATE INDEX IF NOT EXISTS evidence_attempts_obligation_idx ON evidence_attempts(obligation_id, created_at);
CREATE INDEX IF NOT EXISTS evidence_attempt_artifacts_attempt_idx ON evidence_attempt_artifacts(attempt_id);
CREATE INDEX IF NOT EXISTS evidence_receipts_obligation_idx ON evidence_receipts(obligation_id, receipt_status, created_at);
CREATE INDEX IF NOT EXISTS evidence_observation_results_type_idx ON evidence_observation_results(observation_type, result_status);
CREATE INDEX IF NOT EXISTS coverage_verdicts_scope_idx ON coverage_verdicts(project_id, scope_kind, scope_id, coverage_status);
CREATE UNIQUE INDEX IF NOT EXISTS coverage_verdicts_history_identity_idx ON coverage_verdicts(
  id, project_id, scope_kind, scope_id, binding_digest, policy_digest, config_digest, coverage_status
);
CREATE INDEX IF NOT EXISTS coverage_verdict_history_scope_idx ON coverage_verdict_history(project_id, scope_kind, scope_id, computed_at);
CREATE INDEX IF NOT EXISTS evidence_waivers_scope_idx ON evidence_waivers(project_id, scope_kind, scope_id, expires_at);
CREATE INDEX IF NOT EXISTS evidence_waivers_approval_idx ON evidence_waivers(approval_item_id);
CREATE INDEX IF NOT EXISTS evidence_waiver_observations_observation_idx ON evidence_waiver_observations(observation_id);
CREATE TRIGGER IF NOT EXISTS proof_obligations_no_update
BEFORE UPDATE ON proof_obligations
BEGIN
  SELECT RAISE(ABORT, 'proof_obligations are immutable');
END;
CREATE TRIGGER IF NOT EXISTS proof_obligations_no_delete
BEFORE DELETE ON proof_obligations
BEGIN
  SELECT RAISE(ABORT, 'proof_obligations are immutable');
END;
CREATE TRIGGER IF NOT EXISTS proof_obligations_match_plan_project
BEFORE INSERT ON proof_obligations
WHEN NOT EXISTS (
  SELECT 1
  FROM plans
  WHERE id = NEW.plan_id
    AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'proof_obligations plan_id must belong to project_id');
END;
CREATE TRIGGER IF NOT EXISTS proof_obligations_match_item_project_plan
BEFORE INSERT ON proof_obligations
WHEN NEW.item_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1
    FROM items
    JOIN plans ON plans.id = NEW.plan_id
    WHERE items.id = NEW.item_id
      AND items.project_id = NEW.project_id
      AND items.project_id = plans.project_id
      AND items.plan_path = plans.path
  )
BEGIN
  SELECT RAISE(ABORT, 'proof_obligations item_id must belong to project_id and plan_id');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempts_no_update
BEFORE UPDATE ON evidence_attempts
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempts_no_delete
BEFORE DELETE ON evidence_attempts
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_receipts_no_update
BEFORE UPDATE ON evidence_receipts
BEGIN
  SELECT RAISE(ABORT, 'evidence_receipts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_receipts_no_delete
BEFORE DELETE ON evidence_receipts
BEGIN
  SELECT RAISE(ABORT, 'evidence_receipts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_validated_imports_no_update
BEFORE UPDATE ON evidence_validated_imports
BEGIN
  SELECT RAISE(ABORT, 'evidence_validated_imports are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_validated_imports_no_delete
BEFORE DELETE ON evidence_validated_imports
BEGIN
  SELECT RAISE(ABORT, 'evidence_validated_imports are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_validated_imports_validate_proposal
BEFORE INSERT ON evidence_validated_imports
WHEN planr_evidence_validated_import_is_valid(NEW.id, NEW.digest, NEW.proposal_json) != 1
BEGIN
  SELECT RAISE(ABORT, 'evidence_validated_imports proposal digest mismatch');
END;
"#,
    )?;
    validate_existing_proof_obligation_containment(conn)?;
    migrate_evidence_waivers_scope_kind_goal(conn)?;
    ensure_column(
        conn,
        "verification_capability_instances",
        "manifest_digest",
        "TEXT",
    )?;
    ensure_column(
        conn,
        "evidence_attempts",
        "retry_predecessor_attempt_id",
        "TEXT REFERENCES evidence_attempts(id)",
    )?;
    refresh_evidence_attempt_status_result_invariant(conn)?;
    backfill_capability_instance_manifest_digest(conn)?;
    conn.execute_batch(
        r#"
CREATE TRIGGER IF NOT EXISTS verification_capability_manifests_no_update
BEFORE UPDATE ON verification_capability_manifests
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_manifests are immutable');
END;
CREATE TRIGGER IF NOT EXISTS verification_capability_manifests_no_delete
BEFORE DELETE ON verification_capability_manifests
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_manifests are immutable');
END;
CREATE TRIGGER IF NOT EXISTS verification_capability_instances_require_manifest_digest
BEFORE INSERT ON verification_capability_instances
WHEN NEW.manifest_digest IS NULL
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_instances.manifest_digest is required');
END;
CREATE TRIGGER IF NOT EXISTS verification_capability_instances_match_manifest_digest
BEFORE INSERT ON verification_capability_instances
WHEN NOT EXISTS (
  SELECT 1
  FROM verification_capability_manifests
  WHERE id = NEW.manifest_id
    AND version = NEW.manifest_version
    AND manifest_digest = NEW.manifest_digest
)
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_instances.manifest_digest must match manifest snapshot');
END;
CREATE TRIGGER IF NOT EXISTS verification_capability_instances_no_update
BEFORE UPDATE ON verification_capability_instances
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_instances are immutable');
END;
CREATE TRIGGER IF NOT EXISTS verification_capability_instances_no_delete
BEFORE DELETE ON verification_capability_instances
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_instances are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempts_require_bindings
BEFORE INSERT ON evidence_attempts
WHEN NEW.obligation_id IS NULL OR NEW.capability_instance_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts require obligation_id and capability_instance_id');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempts_match_obligation_project
BEFORE INSERT ON evidence_attempts
WHEN NOT EXISTS (
  SELECT 1
  FROM proof_obligations
  WHERE id = NEW.obligation_id
    AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts obligation_id must belong to project_id');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempts_status_result_invariant
BEFORE INSERT ON evidence_attempts
WHEN NEW.completed_at IS NULL
  OR julianday(NEW.completed_at) < julianday(NEW.started_at)
  OR json_extract(NEW.attempt_json, '$.status') IS NOT NEW.attempt_status
  OR COALESCE(CASE NEW.attempt_status
    WHEN 'passed' THEN NOT (
      NEW.exit_code IS 0
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') IS 0
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.error') = 'null'
    )
    WHEN 'failed' THEN NOT (
      NEW.exit_code IS NOT NULL
      AND NEW.exit_code > 0
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') IS NOT NULL
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') > 0
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.error') IS NOT NULL
    )
    WHEN 'timed_out' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'timed_out'
    )
    WHEN 'skipped' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'skipped'
    )
    WHEN 'unavailable' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'unavailable'
    )
    WHEN 'aborted' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND length(json_extract(NEW.attempt_json, '$.exit.signal')) > 0
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'aborted'
    )
    WHEN 'inconclusive' THEN NOT (
      NEW.exit_code = 0
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') = 0
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'inconclusive'
    )
    ELSE 1
  END, 1)
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts status must match execution result');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempts_retry_lineage_matches_predecessor
BEFORE INSERT ON evidence_attempts
WHEN (NEW.retry_predecessor_attempt_id IS NOT NULL OR json_type(NEW.attempt_json, '$.retry_lineage') IS NOT NULL)
  AND (
    json_type(NEW.attempt_json, '$.retry_lineage.previous_attempt_ids') <> 'array'
    OR COALESCE(json_extract(NEW.attempt_json, '$.retry_lineage.max_attempts'), 0)
      < COALESCE(json_extract(NEW.attempt_json, '$.retry_lineage.attempt_number'), 0)
    OR (
      NEW.retry_predecessor_attempt_id IS NULL
      AND (
        COALESCE(json_extract(NEW.attempt_json, '$.retry_lineage.attempt_number'), -1) <> 1
        OR json_array_length(NEW.attempt_json, '$.retry_lineage.previous_attempt_ids') <> 0
      )
    )
    OR (
      NEW.retry_predecessor_attempt_id IS NOT NULL
      AND NOT EXISTS (
        SELECT 1
        FROM evidence_attempts AS predecessor
        WHERE predecessor.id = NEW.retry_predecessor_attempt_id
          AND predecessor.project_id = NEW.project_id
          AND predecessor.obligation_id = NEW.obligation_id
          AND predecessor.capability_instance_id = NEW.capability_instance_id
          AND json_type(predecessor.attempt_json, '$.retry_lineage.previous_attempt_ids') = 'array'
          AND json_extract(NEW.attempt_json, '$.retry_lineage.attempt_number')
            = json_extract(predecessor.attempt_json, '$.retry_lineage.attempt_number') + 1
          AND json_extract(NEW.attempt_json, '$.retry_lineage.max_attempts')
            = json_extract(predecessor.attempt_json, '$.retry_lineage.max_attempts')
          AND json_array_length(NEW.attempt_json, '$.retry_lineage.previous_attempt_ids')
            = json_extract(NEW.attempt_json, '$.retry_lineage.attempt_number') - 1
          AND json_extract(NEW.attempt_json, '$.retry_lineage.previous_attempt_ids')
            = json_insert(
              json_extract(predecessor.attempt_json, '$.retry_lineage.previous_attempt_ids'),
              '$[#]',
              NEW.retry_predecessor_attempt_id
            )
      )
    )
  )
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts retry lineage must reference the prior same-scope attempt');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempt_artifacts_require_concrete_reference
BEFORE INSERT ON evidence_attempt_artifacts
WHEN NEW.artifact_id IS NULL AND NEW.path IS NULL
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempt_artifacts require path or artifact_id');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempt_artifacts_match_attempt_project
BEFORE INSERT ON evidence_attempt_artifacts
WHEN NEW.artifact_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1
    FROM evidence_attempts AS attempts
    JOIN artifacts ON artifacts.id = NEW.artifact_id
    WHERE attempts.id = NEW.attempt_id
      AND artifacts.project_id = attempts.project_id
  )
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempt_artifacts artifact_id must belong to attempt project_id');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempt_artifacts_match_artifact_path
BEFORE INSERT ON evidence_attempt_artifacts
WHEN NEW.artifact_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1
    FROM artifacts
    WHERE id = NEW.artifact_id
      AND path IS NOT NULL
      AND (NEW.path IS NULL OR path = NEW.path)
  )
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempt_artifacts artifact_id path must match supplied path');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempt_artifacts_path_contained
BEFORE INSERT ON evidence_attempt_artifacts
WHEN (NEW.artifact_id IS NOT NULL OR NEW.path IS NOT NULL)
  AND NOT EXISTS (
  SELECT 1
  FROM evidence_attempts AS attempts
  JOIN projects ON projects.id = attempts.project_id
  LEFT JOIN artifacts ON artifacts.id = NEW.artifact_id
  WHERE attempts.id = NEW.attempt_id
    AND COALESCE(NEW.path, artifacts.path) IS NOT NULL
    AND planr_evidence_artifact_path_is_contained(
      projects.root_path,
      NEW.root_kind,
      COALESCE(NEW.path, artifacts.path)
    ) = 1
)
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempt_artifacts path must stay inside its root');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempt_artifacts_no_update
BEFORE UPDATE ON evidence_attempt_artifacts
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempt_artifacts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_attempt_artifacts_no_delete
BEFORE DELETE ON evidence_attempt_artifacts
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempt_artifacts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_receipts_require_obligation
BEFORE INSERT ON evidence_receipts
WHEN NEW.obligation_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'evidence_receipts.obligation_id is required');
END;
CREATE TRIGGER IF NOT EXISTS evidence_receipts_match_attempt_identity
BEFORE INSERT ON evidence_receipts
WHEN NOT EXISTS (
  SELECT 1
  FROM evidence_attempts
  WHERE id = NEW.attempt_id
    AND project_id = NEW.project_id
    AND obligation_id = NEW.obligation_id
)
BEGIN
  SELECT RAISE(ABORT, 'evidence_receipts must match attempt project_id and obligation_id');
END;
CREATE TRIGGER IF NOT EXISTS evidence_receipts_validate_trusted_binding
BEFORE INSERT ON evidence_receipts
WHEN planr_evidence_receipt_binding_is_valid(NEW.trusted_binding_json, NEW.receipt_json) != 1
BEGIN
  SELECT RAISE(ABORT, 'evidence_receipts trusted binding must exactly match receipt');
END;
CREATE TRIGGER IF NOT EXISTS evidence_observation_results_no_update
BEFORE UPDATE ON evidence_observation_results
BEGIN
  SELECT RAISE(ABORT, 'evidence_observation_results are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_observation_results_no_delete
BEFORE DELETE ON evidence_observation_results
BEGIN
  SELECT RAISE(ABORT, 'evidence_observation_results are immutable');
END;
CREATE TRIGGER IF NOT EXISTS coverage_verdict_history_match_verdict_identity
BEFORE INSERT ON coverage_verdict_history
WHEN NOT EXISTS (
  SELECT 1
  FROM coverage_verdicts
  WHERE id = NEW.verdict_id
    AND project_id = NEW.project_id
    AND scope_kind = NEW.scope_kind
    AND scope_id = NEW.scope_id
    AND binding_digest = NEW.binding_digest
    AND policy_digest = NEW.policy_digest
    AND config_digest = NEW.config_digest
    AND coverage_status = NEW.coverage_status
)
BEGIN
  SELECT RAISE(ABORT, 'coverage_verdict_history must snapshot its parent verdict identity');
END;
CREATE TRIGGER IF NOT EXISTS coverage_verdict_history_no_update
BEFORE UPDATE ON coverage_verdict_history
BEGIN
  SELECT RAISE(ABORT, 'coverage_verdict_history is immutable');
END;
CREATE TRIGGER IF NOT EXISTS coverage_verdict_history_no_delete
BEFORE DELETE ON coverage_verdict_history
BEGIN
  SELECT RAISE(ABORT, 'coverage_verdict_history is immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_require_approved_item
BEFORE INSERT ON evidence_waivers
WHEN NOT EXISTS (
  SELECT 1
  FROM items
  WHERE id = NEW.approval_item_id
    AND project_id = NEW.project_id
    AND approval_status = 'approved'
)
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers require a same-project approved item');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_require_exact_observation
BEFORE INSERT ON evidence_waivers
WHEN NEW.obligation_id IS NULL
  OR NEW.observation_id IS NULL
  OR trim(NEW.observation_id) = ''
  OR (NEW.scope_kind = 'observation' AND NEW.scope_id <> NEW.observation_id)
  OR NOT EXISTS (
    SELECT 1
    FROM proof_obligations AS obligations,
         json_each(obligations.observation_requirements_json) AS observations
    WHERE obligations.id = NEW.obligation_id
      AND obligations.project_id = NEW.project_id
      AND (
        observations.value = NEW.observation_id
        OR (
          json_valid(observations.value)
          AND json_extract(observations.value, '$.id') = NEW.observation_id
        )
      )
  )
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers require an exact obligation observation scope');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_validate_contract_json_is_valid
BEFORE INSERT ON evidence_waivers
WHEN json_valid(NEW.waiver_json) <> 1
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers waiver_json must match the frozen waiver contract');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_validate_contract_json
BEFORE INSERT ON evidence_waivers
WHEN json_valid(NEW.waiver_json) = 1
  AND (
  planr_evidence_waiver_contract_is_valid(
    NEW.waiver_json,
    NEW.id,
    NEW.approval_item_id,
    NEW.scope_kind,
    NEW.scope_id,
    NEW.reason,
    NEW.created_by,
    NEW.expires_at,
    NEW.created_at
  ) <> 1
  OR
  json_type(NEW.waiver_json, '$') IS NOT 'object'
  OR json_extract(NEW.waiver_json, '$.id') IS NOT NEW.id
  OR json_extract(NEW.waiver_json, '$.schema_version') IS NOT 'evidence.contract.v1'
  OR json_extract(NEW.waiver_json, '$.approval_ref') IS NOT NEW.approval_item_id
  OR json_extract(NEW.waiver_json, '$.scope.kind') IS NOT NEW.scope_kind
  OR json_extract(NEW.waiver_json, '$.scope.id') IS NOT NEW.scope_id
  OR json_extract(NEW.waiver_json, '$.reason') IS NOT NEW.reason
  OR json_extract(NEW.waiver_json, '$.created_by') IS NOT NEW.created_by
  OR json_extract(NEW.waiver_json, '$.expires_at') IS NOT NEW.expires_at
  OR json_type(NEW.waiver_json, '$.source') IS NOT 'object'
  OR json_type(NEW.waiver_json, '$.target') IS NOT 'object'
  OR json_type(NEW.waiver_json, '$.observation_ids') IS NOT 'array'
  OR COALESCE(json_array_length(NEW.waiver_json, '$.observation_ids'), 0) < 1
  OR json_type(NEW.waiver_json, '$.audit_trail') IS NOT 'array'
  OR COALESCE(json_array_length(NEW.waiver_json, '$.audit_trail'), 0) < 1
  OR NOT EXISTS (
    SELECT 1
    FROM json_each(NEW.waiver_json, '$.observation_ids') AS waived_observations
    WHERE waived_observations.value = NEW.observation_id
  )
  OR EXISTS (
    SELECT 1
    FROM json_each(NEW.waiver_json, '$.observation_ids') AS waived_observations
    WHERE waived_observations.type <> 'text'
       OR trim(waived_observations.value) = ''
       OR NOT EXISTS (
         SELECT 1
         FROM proof_obligations AS obligations,
              json_each(obligations.observation_requirements_json) AS required_observations
         WHERE obligations.id = NEW.obligation_id
           AND obligations.project_id = NEW.project_id
           AND (
             required_observations.value = waived_observations.value
             OR (
               json_valid(required_observations.value)
               AND json_extract(required_observations.value, '$.id') = waived_observations.value
             )
           )
       )
  )
  )
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers waiver_json must match the frozen waiver contract');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_populate_observations
AFTER INSERT ON evidence_waivers
BEGIN
  INSERT INTO evidence_waiver_observations(waiver_id, observation_id, ordinal, created_at)
  SELECT NEW.id, waived_observations.value, waived_observations.key, NEW.created_at
  FROM json_each(NEW.waiver_json, '$.observation_ids') AS waived_observations;
END;
CREATE TRIGGER IF NOT EXISTS evidence_waiver_observations_validate_parent_json
BEFORE INSERT ON evidence_waiver_observations
WHEN NOT EXISTS (
  SELECT 1
  FROM evidence_waivers AS waivers,
       json_each(waivers.waiver_json, '$.observation_ids') AS waived_observations
  WHERE waivers.id = NEW.waiver_id
    AND waived_observations.key = NEW.ordinal
    AND waived_observations.value = NEW.observation_id
)
BEGIN
  SELECT RAISE(ABORT, 'evidence_waiver_observations must match parent waiver_json observation_ids exactly');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waiver_observations_no_update
BEFORE UPDATE ON evidence_waiver_observations
BEGIN
  SELECT RAISE(ABORT, 'evidence_waiver_observations are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waiver_observations_no_delete
BEFORE DELETE ON evidence_waiver_observations
BEGIN
  SELECT RAISE(ABORT, 'evidence_waiver_observations are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_approval_no_update
BEFORE UPDATE OF project_id, approval_item_id ON evidence_waivers
WHEN NOT EXISTS (
  SELECT 1
  FROM items
  WHERE id = NEW.approval_item_id
    AND project_id = NEW.project_id
    AND approval_status = 'approved'
)
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers require a same-project approved item');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_observation_no_update
BEFORE UPDATE OF project_id, obligation_id, observation_id, scope_kind, scope_id ON evidence_waivers
WHEN NEW.obligation_id IS NULL
  OR NEW.observation_id IS NULL
  OR trim(NEW.observation_id) = ''
  OR (NEW.scope_kind = 'observation' AND NEW.scope_id <> NEW.observation_id)
  OR NOT EXISTS (
    SELECT 1
    FROM proof_obligations AS obligations,
         json_each(obligations.observation_requirements_json) AS observations
    WHERE obligations.id = NEW.obligation_id
      AND obligations.project_id = NEW.project_id
      AND (
        observations.value = NEW.observation_id
        OR (
          json_valid(observations.value)
          AND json_extract(observations.value, '$.id') = NEW.observation_id
        )
      )
  )
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers require an exact obligation observation scope');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_no_update
BEFORE UPDATE ON evidence_waivers
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers are immutable');
END;
CREATE TRIGGER IF NOT EXISTS evidence_waivers_no_delete
BEFORE DELETE ON evidence_waivers
BEGIN
  SELECT RAISE(ABORT, 'evidence_waivers are immutable');
END;
"#,
    )?;
    backfill_evidence_waiver_observations(conn)?;
    validate_existing_proof_obligation_containment(conn)?;
    ensure_proof_obligations_canonical_indexes(conn)?;
    ensure_column(
        conn,
        "proof_obligations",
        "retry_aggregation",
        "TEXT NOT NULL DEFAULT 'latest_applicable_pass' CHECK(retry_aggregation IN ('latest_applicable_pass','all_applicable_pass'))",
    )?;
    Ok(())
}

struct CanonicalIndex {
    name: &'static str,
    table: &'static str,
    unique: bool,
    origin: &'static str,
    partial: bool,
    sql: &'static str,
    xinfo: &'static [&'static str],
}

const PROOF_OBLIGATIONS_CANONICAL_INDEXES: [CanonicalIndex; 2] = [
    CanonicalIndex {
        name: "proof_obligations_item_idx",
        table: "proof_obligations",
        unique: false,
        origin: "c",
        partial: false,
        sql: "CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id)",
        xinfo: &[
            "seqno=0|cid=3|name=item_id|desc=0|coll=BINARY|key=1",
            "seqno=1|cid=-1|name=NULL|desc=0|coll=BINARY|key=0",
        ],
    },
    CanonicalIndex {
        name: "proof_obligations_project_identity_idx",
        table: "proof_obligations",
        unique: true,
        origin: "c",
        partial: false,
        sql: "CREATE UNIQUE INDEX proof_obligations_project_identity_idx ON proof_obligations(id, project_id)",
        xinfo: &[
            "seqno=0|cid=0|name=id|desc=0|coll=BINARY|key=1",
            "seqno=1|cid=1|name=project_id|desc=0|coll=BINARY|key=1",
            "seqno=2|cid=-1|name=NULL|desc=0|coll=BINARY|key=0",
        ],
    },
];

fn ensure_proof_obligations_canonical_indexes(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "proof_obligations")? {
        return Ok(());
    }
    conn.execute_batch("SAVEPOINT proof_obligations_canonical_indexes")?;
    let result = (|| -> Result<()> {
        for index in PROOF_OBLIGATIONS_CANONICAL_INDEXES {
            match index_contract(conn, index.name)? {
                Some(existing) if existing.matches(&index) => continue,
                Some(_) => {
                    conn.execute(&format!("DROP INDEX {}", index.name), [])?;
                }
                None => {}
            }
            conn.execute(index.sql, [])?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("RELEASE proof_obligations_canonical_indexes")?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO proof_obligations_canonical_indexes; RELEASE proof_obligations_canonical_indexes",
            );
            Err(error)
        }
    }
}

struct IndexContract {
    table: String,
    sql: String,
    unique: bool,
    origin: String,
    partial: bool,
    xinfo: Vec<String>,
}

impl IndexContract {
    fn matches(&self, expected: &CanonicalIndex) -> bool {
        self.table == expected.table
            && self.sql == normalize_schema_sql(expected.sql)
            && self.unique == expected.unique
            && self.origin == expected.origin
            && self.partial == expected.partial
            && self.xinfo == expected.xinfo
    }
}

fn index_contract(conn: &Connection, name: &str) -> Result<Option<IndexContract>> {
    let row = conn
        .query_row(
            "SELECT tbl_name, COALESCE(sql, '') FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [name],
            |row| Ok((row.get::<_, String>(0)?, normalize_schema_sql(&row.get::<_, String>(1)?))),
        )
        .optional()?;
    let Some((table, sql)) = row else {
        return Ok(None);
    };
    let mut list_stmt = conn.prepare(&format!("PRAGMA index_list('{table}')"))?;
    let list_rows = list_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let Some((_, unique, origin, partial)) = list_rows
        .into_iter()
        .find(|(index_name, _, _, _)| index_name == name)
    else {
        return Ok(None);
    };
    let xinfo = index_xinfo_contract(conn, name)?;
    Ok(Some(IndexContract {
        table,
        sql,
        unique,
        origin,
        partial,
        xinfo,
    }))
}

fn index_xinfo_contract(conn: &Connection, name: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA index_xinfo('{name}')"))?;
    stmt.query_map([], |row| {
        Ok(format!(
            "seqno={}|cid={}|name={}|desc={}|coll={}|key={}",
            sqlite_value_ref_string(row.get_ref(0)?),
            sqlite_value_ref_string(row.get_ref(1)?),
            sqlite_value_ref_string(row.get_ref(2)?),
            sqlite_value_ref_string(row.get_ref(3)?),
            sqlite_value_ref_string(row.get_ref(4)?),
            sqlite_value_ref_string(row.get_ref(5)?)
        ))
    })?
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(Into::into)
}

fn sqlite_value_ref_string(value: rusqlite::types::ValueRef<'_>) -> String {
    match value {
        rusqlite::types::ValueRef::Null => "NULL".to_string(),
        rusqlite::types::ValueRef::Integer(value) => value.to_string(),
        rusqlite::types::ValueRef::Real(value) => value.to_string(),
        rusqlite::types::ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        rusqlite::types::ValueRef::Blob(value) => format!("<blob:{}>", value.len()),
    }
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_existing_proof_obligation_containment(conn: &Connection) -> Result<()> {
    let invalid = conn
        .query_row(
            "SELECT obligations.id
             FROM proof_obligations AS obligations
             LEFT JOIN plans
               ON plans.id = obligations.plan_id
              AND plans.project_id = obligations.project_id
             LEFT JOIN items
               ON items.id = obligations.item_id
              AND items.project_id = obligations.project_id
              AND items.plan_path = plans.path
             WHERE plans.id IS NULL
                OR (obligations.item_id IS NOT NULL AND items.id IS NULL)
             ORDER BY obligations.id
             LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(id) = invalid {
        bail!("proof_obligations containment upgrade blocked by invalid historical row: {id}");
    }
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(Into::into)
}

pub(super) fn register_connection_functions(conn: &Connection) -> Result<()> {
    register_artifact_path_validator(conn)?;
    register_evidence_waiver_contract_validator(conn)?;
    register_validated_import_contract_validator(conn)?;
    register_evidence_receipt_binding_validator(conn)
}

fn register_artifact_path_validator(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "planr_evidence_artifact_path_is_contained",
        3,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let root_path: String = ctx.get(0)?;
            let root_kind: String = ctx.get(1)?;
            let path: String = ctx.get(2)?;
            Ok(
                if artifact_path_is_contained(&root_path, &root_kind, &path) {
                    1i64
                } else {
                    0i64
                },
            )
        },
    )?;
    Ok(())
}

fn register_evidence_waiver_contract_validator(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "planr_evidence_waiver_contract_is_valid",
        9,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let waiver_json: String = ctx.get(0)?;
            let id: String = ctx.get(1)?;
            let approval_ref: String = ctx.get(2)?;
            let scope_kind: String = ctx.get(3)?;
            let scope_id: String = ctx.get(4)?;
            let reason: String = ctx.get(5)?;
            let created_by: String = ctx.get(6)?;
            let expires_at: String = ctx.get(7)?;
            let created_at: String = ctx.get(8)?;
            let row = EvidenceWaiverContractRow {
                id: &id,
                approval_ref: &approval_ref,
                scope_kind: &scope_kind,
                scope_id: &scope_id,
                reason: &reason,
                created_by: &created_by,
                expires_at: &expires_at,
                created_at: &created_at,
            };
            Ok(if evidence_waiver_contract_is_valid(&waiver_json, row) {
                1i64
            } else {
                0i64
            })
        },
    )?;
    Ok(())
}

fn register_validated_import_contract_validator(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "planr_evidence_validated_import_is_valid",
        3,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let id: String = ctx.get(0)?;
            let digest: String = ctx.get(1)?;
            let proposal_json: String = ctx.get(2)?;
            Ok(
                if validated_import_contract_is_valid(&id, &digest, &proposal_json) {
                    1i64
                } else {
                    0i64
                },
            )
        },
    )?;
    Ok(())
}

fn register_evidence_receipt_binding_validator(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "planr_evidence_receipt_binding_is_valid",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let trusted_binding_json: String = ctx.get(0)?;
            let receipt_json: String = ctx.get(1)?;
            Ok(
                if evidence_receipt_binding_is_valid(&trusted_binding_json, &receipt_json) {
                    1i64
                } else {
                    0i64
                },
            )
        },
    )?;
    Ok(())
}

fn evidence_receipt_binding_is_valid(trusted_binding_json: &str, receipt_json: &str) -> bool {
    let Ok(binding) = serde_json::from_str::<Value>(trusted_binding_json) else {
        return false;
    };
    let Ok(receipt) = serde_json::from_str::<Value>(receipt_json) else {
        return false;
    };
    trusted_receipt_binding_matches_receipt(binding, receipt).is_ok()
}

fn validated_import_contract_is_valid(id: &str, digest: &str, proposal_json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(proposal_json) else {
        return false;
    };
    let Ok(proposal) = parse_untrusted_evidence_proposal(value.clone()) else {
        return false;
    };
    if proposal.id != id {
        return false;
    }
    sha256_json_digest(&value).is_ok_and(|actual| actual == digest)
}

fn artifact_path_is_contained(root_path: &str, root_kind: &str, path: &str) -> bool {
    let Some(root) = artifact_root(root_path, root_kind) else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let relative = Path::new(path);
    if !relative.is_relative() {
        return false;
    }
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return false;
    }

    let candidate = root.join(relative);
    let Ok(canonical) = candidate.canonicalize() else {
        return false;
    };
    canonical.starts_with(&root)
}

struct EvidenceWaiverContractRow<'a> {
    id: &'a str,
    approval_ref: &'a str,
    scope_kind: &'a str,
    scope_id: &'a str,
    reason: &'a str,
    created_by: &'a str,
    expires_at: &'a str,
    created_at: &'a str,
}

fn evidence_waiver_contract_is_valid(
    waiver_json: &str,
    row: EvidenceWaiverContractRow<'_>,
) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(waiver_json) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    if !has_exact_keys(
        object,
        &[
            "id",
            "schema_version",
            "scope",
            "observation_ids",
            "source",
            "target",
            "reason",
            "created_by",
            "created_at",
            "expires_at",
            "approval_ref",
            "audit_trail",
        ],
    ) {
        return false;
    }
    if json_str(object, "id") != Some(row.id)
        || !is_contract_id(row.id)
        || json_str(object, "schema_version") != Some("evidence.contract.v1")
        || json_str(object, "approval_ref") != Some(row.approval_ref)
        || !is_contract_id(row.approval_ref)
        || json_str(object, "reason") != Some(row.reason)
        || row.reason.is_empty()
        || json_str(object, "created_by") != Some(row.created_by)
        || row.created_by.is_empty()
        || json_str(object, "created_at") != Some(row.created_at)
        || !is_rfc3339(row.created_at)
        || json_str(object, "expires_at") != Some(row.expires_at)
        || !is_rfc3339(row.expires_at)
    {
        return false;
    }
    validate_waiver_scope(object.get("scope"), row.scope_kind, row.scope_id)
        && validate_source_binding(object.get("source"))
        && validate_target_binding(object.get("target"))
        && validate_waiver_observation_ids(object.get("observation_ids"))
        && validate_waiver_audit_trail(object.get("audit_trail"))
}

fn validate_waiver_scope(value: Option<&Value>, scope_kind: &str, scope_id: &str) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    if !keys_are_subset(
        object,
        &["kind", "id", "plan_id", "item_id", "criterion_id"],
    ) {
        return false;
    }
    matches!(scope_kind, "criterion" | "item" | "plan" | "goal")
        && json_str(object, "kind") == Some(scope_kind)
        && json_str(object, "id") == Some(scope_id)
        && is_contract_id(scope_id)
        && optional_contract_id(object, "plan_id")
        && optional_contract_id(object, "item_id")
        && optional_contract_id(object, "criterion_id")
}

fn validate_source_binding(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    has_exact_keys(object, &["revision", "tree_digest", "dirty"])
        && object
            .get("revision")
            .and_then(Value::as_str)
            .is_some_and(|value| value.chars().count() >= 7)
        && object
            .get("tree_digest")
            .and_then(Value::as_str)
            .is_some_and(is_sha256_digest)
        && object.get("dirty").and_then(Value::as_bool).is_some()
}

fn validate_target_binding(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    if !keys_are_subset(object, &["kind", "uri", "digest", "deployment_id"]) {
        return false;
    }
    object
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
        && object
            .get("uri")
            .map(|value| value.as_str().is_some_and(|value| !value.is_empty()))
            .unwrap_or(true)
        && object
            .get("digest")
            .map(|value| value.as_str().is_some_and(is_sha256_digest))
            .unwrap_or(true)
        && object
            .get("deployment_id")
            .map(|value| value.as_str().is_some_and(is_contract_id))
            .unwrap_or(true)
        && (object.contains_key("uri")
            || object.contains_key("digest")
            || object.contains_key("deployment_id"))
}

fn validate_waiver_observation_ids(value: Option<&Value>) -> bool {
    let Some(values) = value.and_then(Value::as_array) else {
        return false;
    };
    !values.is_empty()
        && values
            .iter()
            .all(|value| value.as_str().is_some_and(is_contract_id))
}

fn validate_waiver_audit_trail(value: Option<&Value>) -> bool {
    let Some(values) = value.and_then(Value::as_array) else {
        return false;
    };
    !values.is_empty() && values.iter().all(Value::is_object)
}

fn has_exact_keys(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.len() == keys.len()
        && keys_are_subset(object, keys)
        && keys.iter().all(|key| object.contains_key(*key))
}

fn keys_are_subset(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.keys().all(|key| keys.contains(&key.as_str()))
}

fn optional_contract_id(object: &Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .map(|value| value.as_str().is_some_and(is_contract_id))
        .unwrap_or(true)
}

fn refresh_evidence_attempt_status_result_invariant(conn: &Connection) -> Result<()> {
    let trigger_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'trigger'
               AND name = 'evidence_attempts_status_result_invariant'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if trigger_sql
        .as_deref()
        .is_some_and(evidence_attempt_status_result_trigger_accepts_unavailable)
    {
        return Ok(());
    }
    conn.execute_batch(
        r#"
DROP TRIGGER IF EXISTS evidence_attempts_status_result_invariant;
CREATE TRIGGER evidence_attempts_status_result_invariant
BEFORE INSERT ON evidence_attempts
WHEN NEW.completed_at IS NULL
  OR julianday(NEW.completed_at) < julianday(NEW.started_at)
  OR json_extract(NEW.attempt_json, '$.status') IS NOT NEW.attempt_status
  OR COALESCE(CASE NEW.attempt_status
    WHEN 'passed' THEN NOT (
      NEW.exit_code IS 0
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') IS 0
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.error') = 'null'
    )
    WHEN 'failed' THEN NOT (
      NEW.exit_code IS NOT NULL
      AND NEW.exit_code > 0
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') IS NOT NULL
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') > 0
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.error') IS NOT NULL
    )
    WHEN 'timed_out' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'timed_out'
    )
    WHEN 'skipped' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'skipped'
    )
    WHEN 'unavailable' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'unavailable'
    )
    WHEN 'aborted' THEN NOT (
      NEW.exit_code IS NULL
      AND json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'
      AND length(json_extract(NEW.attempt_json, '$.exit.signal')) > 0
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'aborted'
    )
    WHEN 'inconclusive' THEN NOT (
      NEW.exit_code = 0
      AND json_extract(NEW.attempt_json, '$.exit.exit_code') = 0
      AND json_type(NEW.attempt_json, '$.exit.signal') = 'null'
      AND json_extract(NEW.attempt_json, '$.exit.error') = 'inconclusive'
    )
    ELSE 1
  END, 1)
BEGIN
  SELECT RAISE(ABORT, 'evidence_attempts status must match execution result');
END;
"#,
    )?;
    Ok(())
}

fn evidence_attempt_status_result_trigger_accepts_unavailable(sql: &str) -> bool {
    sql.contains("julianday(NEW.completed_at) < julianday(NEW.started_at)")
        && sql.contains("WHEN 'unavailable' THEN NOT")
        && sql.contains("json_type(NEW.attempt_json, '$.exit.exit_code') = 'null'")
        && sql.contains("json_type(NEW.attempt_json, '$.exit.signal') = 'null'")
        && sql.contains("json_extract(NEW.attempt_json, '$.exit.error') = 'unavailable'")
}

fn json_str<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn is_contract_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn is_rfc3339(value: &str) -> bool {
    OffsetDateTime::parse(value, &Rfc3339).is_ok()
}

fn artifact_root(root_path: &str, root_kind: &str) -> Option<std::path::PathBuf> {
    let project_root = Path::new(root_path);
    match root_kind {
        "project" => Some(project_root.to_path_buf()),
        "planr" => Some(project_root.join(".planr")),
        "artifact_store" => Some(project_root.join(".planr").join("artifacts")),
        "external" => None,
        _ => None,
    }
}

fn backfill_capability_instance_manifest_digest(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
DROP TRIGGER IF EXISTS verification_capability_instances_no_update;
"#,
    )?;

    let unresolved_instances = query_ids(
        &tx,
        r#"
SELECT instances.id
FROM verification_capability_instances AS instances
LEFT JOIN verification_capability_manifests AS manifests
  ON manifests.id = instances.manifest_id
 AND manifests.version = instances.manifest_version
GROUP BY instances.id
HAVING COUNT(manifests.manifest_digest) <> 1
ORDER BY instances.id
LIMIT 5
"#,
    )?;
    if !unresolved_instances.is_empty() {
        bail!(
            "verification_capability_instances manifest_digest backfill failed: missing or ambiguous manifest for {}",
            unresolved_instances.join(", ")
        );
    }

    tx.execute(
        r#"
UPDATE verification_capability_instances AS instances
SET manifest_digest = (
  SELECT manifests.manifest_digest
  FROM verification_capability_manifests AS manifests
  WHERE manifests.id = instances.manifest_id
    AND manifests.version = instances.manifest_version
)
WHERE manifest_digest IS NULL
"#,
        [],
    )?;

    let mismatched_instances = query_ids(
        &tx,
        r#"
SELECT instances.id
FROM verification_capability_instances AS instances
WHERE instances.manifest_digest IS NULL
   OR NOT EXISTS (
     SELECT 1
     FROM verification_capability_manifests AS manifests
     WHERE manifests.id = instances.manifest_id
       AND manifests.version = instances.manifest_version
       AND manifests.manifest_digest = instances.manifest_digest
   )
ORDER BY instances.id
LIMIT 5
"#,
    )?;
    if !mismatched_instances.is_empty() {
        bail!(
            "verification_capability_instances manifest_digest backfill failed: mismatched digest for {}",
            mismatched_instances.join(", ")
        );
    }

    tx.execute_batch(
        r#"
CREATE TRIGGER IF NOT EXISTS verification_capability_instances_no_update
BEFORE UPDATE ON verification_capability_instances
BEGIN
  SELECT RAISE(ABORT, 'verification_capability_instances are immutable');
END;
"#,
    )?;
    tx.commit()?;
    Ok(())
}

fn migrate_evidence_waivers_scope_kind_goal(conn: &Connection) -> Result<()> {
    let create_sql: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'evidence_waivers'",
        [],
        |row| row.get(0),
    )?;
    if create_sql
        .as_deref()
        .is_some_and(|sql| sql.contains("'goal'"))
    {
        return Ok(());
    }

    conn.execute_batch(
        r#"
PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;
DROP TRIGGER IF EXISTS evidence_waivers_require_approved_item;
DROP TRIGGER IF EXISTS evidence_waivers_require_exact_observation;
DROP TRIGGER IF EXISTS evidence_waivers_validate_contract_json_is_valid;
DROP TRIGGER IF EXISTS evidence_waivers_validate_contract_json;
DROP TRIGGER IF EXISTS evidence_waivers_populate_observations;
DROP TRIGGER IF EXISTS evidence_waiver_observations_validate_parent_json;
DROP TRIGGER IF EXISTS evidence_waiver_observations_no_update;
DROP TRIGGER IF EXISTS evidence_waiver_observations_no_delete;
DROP TRIGGER IF EXISTS evidence_waivers_approval_no_update;
DROP TRIGGER IF EXISTS evidence_waivers_observation_no_update;
DROP TRIGGER IF EXISTS evidence_waivers_no_update;
DROP TRIGGER IF EXISTS evidence_waivers_no_delete;
ALTER TABLE evidence_waiver_observations RENAME TO evidence_waiver_observations_old;
ALTER TABLE evidence_waivers RENAME TO evidence_waivers_old;
CREATE TABLE evidence_waivers(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  approval_item_id TEXT NOT NULL,
  obligation_id TEXT,
  observation_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL CHECK(scope_kind IN ('project','plan','item','criterion','goal','obligation','observation')),
  scope_id TEXT NOT NULL,
  waiver_digest TEXT NOT NULL UNIQUE,
  reason TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_by TEXT NOT NULL,
  waiver_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(approval_item_id) REFERENCES items(id),
  FOREIGN KEY(obligation_id) REFERENCES proof_obligations(id),
  CHECK(reason <> ''),
  CHECK(created_by <> '')
);
INSERT INTO evidence_waivers(
  id, project_id, approval_item_id, obligation_id, observation_id,
  scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
  waiver_json, created_at
)
SELECT
  id, project_id, approval_item_id, obligation_id, observation_id,
  scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
  waiver_json, created_at
FROM evidence_waivers_old;
CREATE TABLE evidence_waiver_observations(
  waiver_id TEXT NOT NULL,
  observation_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  created_at TEXT NOT NULL,
  PRIMARY KEY(waiver_id, observation_id),
  FOREIGN KEY(waiver_id) REFERENCES evidence_waivers(id),
  CHECK(observation_id <> '')
);
INSERT INTO evidence_waiver_observations(waiver_id, observation_id, ordinal, created_at)
SELECT waiver_id, observation_id, ordinal, created_at
FROM evidence_waiver_observations_old;
DROP TABLE evidence_waiver_observations_old;
DROP TABLE evidence_waivers_old;
COMMIT;
PRAGMA foreign_keys = ON;
CREATE INDEX IF NOT EXISTS evidence_waivers_scope_idx ON evidence_waivers(project_id, scope_kind, scope_id, expires_at);
CREATE INDEX IF NOT EXISTS evidence_waivers_approval_idx ON evidence_waivers(approval_item_id);
CREATE INDEX IF NOT EXISTS evidence_waiver_observations_observation_idx ON evidence_waiver_observations(observation_id);
"#,
    )?;
    Ok(())
}

fn backfill_evidence_waiver_observations(conn: &Connection) -> Result<()> {
    let malformed_waivers = query_ids(
        conn,
        r#"
SELECT id
FROM evidence_waivers
WHERE planr_evidence_waiver_contract_is_valid(
  waiver_json,
  id,
  approval_item_id,
  scope_kind,
  scope_id,
  reason,
  created_by,
  expires_at,
  created_at
) <> 1
ORDER BY id
LIMIT 5
"#,
    )?;
    if !malformed_waivers.is_empty() {
        bail!(
            "evidence_waivers legacy waiver_json migration failed: malformed waiver contract for {}",
            malformed_waivers.join(", ")
        );
    }

    conn.execute_batch(
        r#"
INSERT OR IGNORE INTO evidence_waiver_observations(waiver_id, observation_id, ordinal, created_at)
SELECT waivers.id, waived_observations.value, waived_observations.key, waivers.created_at
FROM evidence_waivers AS waivers,
     json_each(
       CASE
         WHEN json_valid(waivers.waiver_json) = 1 THEN waivers.waiver_json
         ELSE '{"observation_ids":[]}'
       END,
       '$.observation_ids'
     ) AS waived_observations
WHERE json_valid(waivers.waiver_json) = 1
  AND json_type(waivers.waiver_json, '$.observation_ids') = 'array'
  AND waived_observations.type = 'text'
  AND trim(waived_observations.value) <> '';
"#,
    )?;

    let non_authoritative_waivers = query_ids(
        conn,
        r#"
SELECT waivers.id
FROM evidence_waivers AS waivers
WHERE NOT EXISTS (
  SELECT 1
  FROM evidence_waiver_observations AS observations
  WHERE observations.waiver_id = waivers.id
    AND observations.observation_id = waivers.observation_id
)
OR EXISTS (
  SELECT 1
  FROM json_each(waivers.waiver_json, '$.observation_ids') AS waived_observations
  WHERE NOT EXISTS (
    SELECT 1
    FROM evidence_waiver_observations AS observations
    WHERE observations.waiver_id = waivers.id
      AND observations.ordinal = waived_observations.key
      AND observations.observation_id = waived_observations.value
  )
)
OR EXISTS (
  SELECT 1
  FROM evidence_waiver_observations AS observations
  WHERE observations.waiver_id = waivers.id
    AND NOT EXISTS (
      SELECT 1
      FROM json_each(waivers.waiver_json, '$.observation_ids') AS waived_observations
      WHERE waived_observations.key = observations.ordinal
        AND waived_observations.value = observations.observation_id
    )
)
OR EXISTS (
  SELECT 1
  FROM evidence_waiver_observations AS observations
  WHERE observations.waiver_id = waivers.id
    AND NOT EXISTS (
      SELECT 1
      FROM proof_obligations AS obligations,
           json_each(obligations.observation_requirements_json) AS required_observations
      WHERE obligations.id = waivers.obligation_id
        AND obligations.project_id = waivers.project_id
        AND (
          required_observations.value = observations.observation_id
          OR (
            json_valid(required_observations.value)
            AND json_extract(required_observations.value, '$.id') = observations.observation_id
          )
        )
    )
)
ORDER BY waivers.id
LIMIT 5
"#,
    )?;
    if !non_authoritative_waivers.is_empty() {
        bail!(
            "evidence_waivers legacy waiver_json migration failed: non-authoritative observations for {}",
            non_authoritative_waivers.join(", ")
        );
    }
    Ok(())
}

fn query_ids(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn ensure_column(conn: &Connection, table: &str, name: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == name {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {name} {definition}"),
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::canonical_json::sha256_json_digest_without_top_level_field;

    use super::super::{ensure_schema, open_db};
    use rusqlite::{Connection, OptionalExtension, params};
    use serde_json::{Value, json};
    use std::fs;

    const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_B: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const DIGEST_C: &str =
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const WAIVER_CREATED_AT: &str = "2026-07-28T12:00:00Z";
    const WAIVER_EXPIRES_AT: &str = "2026-08-01T00:00:00Z";
    const PRIOR_PROOF_OBLIGATIONS_RUNTIME_DDL: &str = r#"
CREATE TABLE proof_obligations(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  item_id TEXT,
  criterion_id TEXT NOT NULL,
  obligation_version INTEGER NOT NULL CHECK(obligation_version >= 1),
  title TEXT NOT NULL,
  binding INTEGER NOT NULL CHECK(binding IN (0, 1)),
  observation_requirements_json TEXT NOT NULL,
  fixture_policy_json TEXT NOT NULL DEFAULT '{}',
  freshness_policy_json TEXT NOT NULL DEFAULT '{}',
  assurance_policy_json TEXT NOT NULL DEFAULT '{}',
  policy_digest TEXT NOT NULL CHECK(policy_digest GLOB 'sha256:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
  config_digest TEXT NOT NULL CHECK(config_digest GLOB 'sha256:[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'),
  source_digest TEXT,
  supersedes_obligation_id TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(item_id) REFERENCES items(id),
  FOREIGN KEY(supersedes_obligation_id) REFERENCES proof_obligations(id)
);
CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id);
CREATE UNIQUE INDEX proof_obligations_project_identity_idx ON proof_obligations(id, project_id);
CREATE INDEX proof_obligations_policy_idx ON proof_obligations(project_id, policy_digest, config_digest);
CREATE TRIGGER proof_obligations_no_update
BEFORE UPDATE ON proof_obligations
BEGIN
  SELECT RAISE(ABORT, 'proof_obligations are immutable');
END;
CREATE TRIGGER proof_obligations_no_delete
BEFORE DELETE ON proof_obligations
BEGIN
  SELECT RAISE(ABORT, 'proof_obligations are immutable');
END;
"#;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        conn
    }

    fn normalize_sql(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ProofObligationRuntimeSchema {
        sqlite_master: Vec<(String, String, String, String)>,
        table_xinfo: Vec<String>,
        foreign_key_list: Vec<String>,
        index_list: Vec<String>,
        index_xinfo: Vec<String>,
    }

    fn sqlite_master_snapshot(conn: &Connection) -> Vec<(String, String, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT type, name, COALESCE(sql, '')
                 FROM sqlite_master
                 WHERE name IN (
                   'meta',
                   'projects',
                   'plans',
                   'items',
                   'proof_obligations',
                   'proof_obligations_item_idx',
                   'proof_obligations_policy_idx',
                   'proof_obligations_project_identity_idx',
                   'proof_obligations_no_update',
                   'proof_obligations_no_delete',
                   'proof_obligations_match_plan_project',
                   'proof_obligations_match_item_project_plan'
                 )
                 ORDER BY type, name",
            )
            .unwrap();
        stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                normalize_sql(&row.get::<_, String>(2)?),
            ))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
    }

    fn proof_obligation_runtime_schema(conn: &Connection) -> ProofObligationRuntimeSchema {
        let mut master_stmt = conn
            .prepare(
                "SELECT type, name, tbl_name, COALESCE(sql, '')
                 FROM sqlite_master
                 WHERE name IN (
                   'proof_obligations',
                   'proof_obligations_item_idx',
                   'proof_obligations_policy_idx',
                   'proof_obligations_project_identity_idx',
                   'proof_obligations_no_update',
                   'proof_obligations_no_delete',
                   'proof_obligations_match_plan_project',
                   'proof_obligations_match_item_project_plan'
                 )
                 ORDER BY type, name",
            )
            .unwrap();
        let sqlite_master = master_stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    normalize_sql(&row.get::<_, String>(3)?),
                ))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        let table_xinfo = pragma_rows(
            conn,
            "PRAGMA table_xinfo('proof_obligations')",
            7,
            &[
                "cid",
                "name",
                "type",
                "notnull",
                "dflt_value",
                "pk",
                "hidden",
            ],
        );
        let foreign_key_list = pragma_rows(
            conn,
            "PRAGMA foreign_key_list('proof_obligations')",
            8,
            &[
                "id",
                "seq",
                "table",
                "from",
                "to",
                "on_update",
                "on_delete",
                "match",
            ],
        );
        let index_list = proof_obligation_index_list_contract(conn);
        let mut index_xinfo = Vec::new();
        for index in [
            "proof_obligations_item_idx",
            "proof_obligations_policy_idx",
            "proof_obligations_project_identity_idx",
        ] {
            for row in pragma_rows(
                conn,
                &format!("PRAGMA index_xinfo('{index}')"),
                6,
                &["seqno", "cid", "name", "desc", "coll", "key"],
            ) {
                index_xinfo.push(format!("{index}|{row}"));
            }
        }
        ProofObligationRuntimeSchema {
            sqlite_master,
            table_xinfo,
            foreign_key_list,
            index_list,
            index_xinfo,
        }
    }

    fn pragma_rows(conn: &Connection, sql: &str, columns: usize, labels: &[&str]) -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        stmt.query_map([], |row| {
            let values = (0..columns)
                .map(|index| {
                    let value = row.get_ref(index)?;
                    Ok(match value {
                        rusqlite::types::ValueRef::Null => "NULL".to_string(),
                        rusqlite::types::ValueRef::Integer(value) => value.to_string(),
                        rusqlite::types::ValueRef::Real(value) => value.to_string(),
                        rusqlite::types::ValueRef::Text(value) => {
                            String::from_utf8_lossy(value).into_owned()
                        }
                        rusqlite::types::ValueRef::Blob(value) => {
                            format!("<blob:{}>", value.len())
                        }
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(labels
                .iter()
                .zip(values)
                .map(|(label, value)| format!("{label}={value}"))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
    }

    fn proof_obligation_index_list_contract(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("PRAGMA index_list('proof_obligations')")
            .unwrap();
        let mut rows = stmt
            .query_map([], |row| {
                Ok(format!(
                    "name={}|unique={}|origin={}|partial={}",
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?
                ))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect::<Vec<_>>();
        rows.sort();
        rows
    }

    fn proof_obligation_index_columns(conn: &Connection, name: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA index_info('{name}')"))
            .unwrap();
        stmt.query_map([], |row| row.get(2))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    fn current_proof_obligation_schema_snapshot() -> Vec<(String, String, String)> {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        sqlite_master_snapshot(&conn)
    }

    fn current_proof_obligation_runtime_schema() -> ProofObligationRuntimeSchema {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        proof_obligation_runtime_schema(&conn)
    }

    fn sqlite_master_entry<'a>(
        snapshot: &'a [(String, String, String)],
        name: &str,
    ) -> &'a (String, String, String) {
        snapshot
            .iter()
            .find(|(_, row_name, _)| row_name == name)
            .unwrap_or_else(|| panic!("missing sqlite_master entry: {name}"))
    }

    fn assert_current_proof_obligation_runtime_schema(conn: &Connection) {
        let fresh = current_proof_obligation_runtime_schema();
        let upgraded = proof_obligation_runtime_schema(conn);
        assert_eq!(upgraded, fresh);
        let upgraded_master = sqlite_master_snapshot(conn);
        let fresh_master = current_proof_obligation_schema_snapshot();
        assert_eq!(
            sqlite_master_entry(&upgraded_master, "proof_obligations"),
            sqlite_master_entry(&fresh_master, "proof_obligations"),
            "proof_obligations table SQL drifted"
        );
        let table_sql = &sqlite_master_entry(&upgraded_master, "proof_obligations").2;
        for invariant in [
            "policy_digest TEXT NOT NULL CHECK(policy_digest GLOB 'sha256:",
            "config_digest TEXT NOT NULL CHECK(config_digest GLOB 'sha256:",
            "FOREIGN KEY(project_id) REFERENCES projects(id)",
            "FOREIGN KEY(item_id) REFERENCES items(id)",
            "FOREIGN KEY(supersedes_obligation_id) REFERENCES proof_obligations(id)",
        ] {
            assert!(
                table_sql.contains(invariant),
                "proof_obligations table missing invariant {invariant}: {table_sql}"
            );
        }
        assert_eq!(
            proof_obligation_index_columns(conn, "proof_obligations_item_idx"),
            vec!["item_id"]
        );
        assert_eq!(
            proof_obligation_index_columns(conn, "proof_obligations_project_identity_idx"),
            vec!["id", "project_id"]
        );
    }

    fn proof_obligation_row_snapshot(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT id || '|' || project_id || '|' || plan_id || '|' ||
                        COALESCE(item_id, '') || '|' || criterion_id || '|' ||
                        obligation_version || '|' || title || '|' || binding || '|' ||
                        observation_requirements_json || '|' || fixture_policy_json || '|' ||
                        freshness_policy_json || '|' || assurance_policy_json || '|' ||
                        policy_digest || '|' || config_digest || '|' ||
                        COALESCE(source_digest, '') || '|' ||
                        COALESCE(supersedes_obligation_id, '') || '|' || created_at
                 FROM proof_obligations
                 ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    fn schema_version_value(conn: &Connection) -> Option<String> {
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
    }

    fn seed_project_and_item(conn: &Connection) {
        seed_project_and_item_with_root(conn, ".");
    }

    fn seed_project_and_item_with_root(conn: &Connection, root_path: &str) {
        conn.execute_batch(
            &format!(
                "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
                 VALUES ('p-evidence', 'Evidence', '{}', 'active', datetime('now'), datetime('now'));
                 INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
                 VALUES ('pln-evidence', 'p-evidence', 'build', '/tmp/planr/evidence.plan.md', 'Evidence Plan', 'evidence-plan', 'ok', 'hash-evidence', datetime('now'), datetime('now'));
                 INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('i-approval', 'p-evidence', 'Approval', 'Approval item', 'ready', 'code', '/tmp/planr/evidence.plan.md', datetime('now'), datetime('now'));",
                root_path.replace('\'', "''")
            ),
        )
        .unwrap();
    }

    fn approve_item(conn: &Connection, item_id: &str) {
        conn.execute(
            "UPDATE items SET approval_status = 'approved', approved_by = 'reviewer' WHERE id = ?1",
            [item_id],
        )
        .unwrap();
    }

    fn seed_binding(conn: &Connection) {
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, created_at
            ) VALUES (
              'obl-1', 'p-evidence', 'pln-evidence', 'i-approval', 'crit-1', 1, 'Obligation',
              1, '[{\"id\":\"obs-http-200\",\"type\":\"example.http.ok\"},{\"id\":\"obs-json-body\",\"type\":\"example.json.body\"}]', '{}', '{}', '{}', ?1, ?2, datetime('now')
            )",
            params![DIGEST, DIGEST_B],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verification_capability_manifests(
              id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, created_at
            ) VALUES ('cap.manifest', '1.0.0', 'process', ?1, ?2, '{}', datetime('now'))",
            params![DIGEST, DIGEST_B],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO verification_capability_instances(
              id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
              availability_status, runtime_target_json, capability_snapshot_json,
              probe_result_json, created_at
            ) VALUES (
              'cap-instance-1', 'cap.manifest', '1.0.0', ?1, 'probe-1',
              'available', '{}', '{}', '{}', datetime('now')
            )",
            params![DIGEST_B],
        )
        .unwrap();
    }

    fn insert_attempt(conn: &Connection) {
        let attempt_json = attempt_json("passed", "0", "null", "null");
        conn.execute(
            "INSERT INTO evidence_attempts(
              id, project_id, obligation_id, capability_instance_id, attempt_status,
              execution_contract_digest, resolved_command_json, environment_digest,
              started_at, completed_at, exit_code, stdout_digest, stderr_digest,
              output_bounds_json, attempt_json, created_at
            ) VALUES (
              'attempt-1', 'p-evidence', 'obl-1', 'cap-instance-1', 'passed',
              ?1, '{\"cmd\":\"true\"}', ?2, datetime('now'), datetime('now'), 0,
              ?3, ?3, '{}', ?4, datetime('now')
            )",
            params![DIGEST, DIGEST_B, DIGEST_C, attempt_json],
        )
        .unwrap();
    }

    fn attempt_json(status: &str, exit_code: &str, signal: &str, error: &str) -> String {
        format!(
            "{{\"status\":\"{status}\",\"exit\":{{\"exit_code\":{exit_code},\"signal\":{signal},\"error\":{error}}}}}"
        )
    }

    fn valid_receipt_and_binding_json() -> (String, String) {
        let receipt = serde_json::from_str::<Value>(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json"
        ))
        .unwrap();
        let binding = json!({
            "source": receipt["source"],
            "target": receipt["target"],
            "environment": receipt["environment"],
            "capability": receipt["capability"],
            "config_digest": receipt["config_digest"],
            "policy_digest": DIGEST,
            "policy_source": "proof_obligation"
        });
        (receipt.to_string(), binding.to_string())
    }

    fn refresh_receipt_digest(receipt: &mut Value) {
        let digest = sha256_json_digest_without_top_level_field(receipt, "receipt_digest").unwrap();
        receipt["receipt_digest"] = json!(digest);
    }

    #[test]
    fn evidence_receipts_validate_trusted_binding_exact_shape_and_fields() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        insert_attempt(&conn);
        let (receipt_json, binding_json) = valid_receipt_and_binding_json();
        let receipt = serde_json::from_str::<Value>(&receipt_json).unwrap();
        let binding = serde_json::from_str::<Value>(&binding_json).unwrap();

        let mut cases = Vec::new();
        cases.push(("exact-success", binding.clone(), receipt.clone(), true));
        for field in [
            "source",
            "target",
            "environment",
            "capability",
            "config_digest",
        ] {
            let mut mismatch = binding.clone();
            mismatch[field] = json!({"mismatched": field});
            cases.push((field, mismatch, receipt.clone(), false));
        }
        for field in [
            "source",
            "target",
            "environment",
            "capability",
            "config_digest",
            "policy_digest",
            "policy_source",
        ] {
            let mut missing = binding.clone();
            missing.as_object_mut().unwrap().remove(field);
            cases.push((field, missing, receipt.clone(), false));
        }
        let mut extra = binding.clone();
        extra["extra"] = json!(true);
        cases.push(("extra-key", extra, receipt.clone(), false));
        for (field, nested) in [
            ("source", "unexpected"),
            ("target", "unexpected"),
            ("environment", "unexpected"),
            ("capability", "unexpected"),
        ] {
            let mut malformed_binding = binding.clone();
            malformed_binding[field][nested] = json!(true);
            let mut malformed_receipt = receipt.clone();
            malformed_receipt[field][nested] = json!(true);
            refresh_receipt_digest(&mut malformed_receipt);
            cases.push((field, malformed_binding, malformed_receipt, false));
        }
        for (field, nested, value) in [
            ("source", "revision", json!("short")),
            ("source", "tree_digest", json!("sha256:not-a-digest")),
            ("source", "dirty", json!("false")),
            ("target", "kind", json!("")),
            ("environment", "id", json!("bad id with spaces")),
            ("environment", "digest", json!("sha256:not-a-digest")),
            ("capability", "manifest_id", json!("bad id with spaces")),
            (
                "capability",
                "manifest_digest",
                json!("sha256:not-a-digest"),
            ),
            ("capability", "instance_id", json!("bad id with spaces")),
            (
                "capability",
                "instance_digest",
                json!("sha256:not-a-digest"),
            ),
        ] {
            let mut malformed_binding = binding.clone();
            malformed_binding[field][nested] = value.clone();
            let mut malformed_receipt = receipt.clone();
            malformed_receipt[field][nested] = value;
            refresh_receipt_digest(&mut malformed_receipt);
            cases.push((nested, malformed_binding, malformed_receipt, false));
        }
        let mut malformed_config_binding = binding.clone();
        malformed_config_binding["config_digest"] = json!("sha256:not-a-digest");
        let mut malformed_config_receipt = receipt.clone();
        malformed_config_receipt["config_digest"] = json!("sha256:not-a-digest");
        refresh_receipt_digest(&mut malformed_config_receipt);
        cases.push((
            "malformed-config-digest",
            malformed_config_binding,
            malformed_config_receipt,
            false,
        ));
        let mut bad_policy_digest = binding.clone();
        bad_policy_digest["policy_digest"] = json!("sha256:not-a-digest");
        cases.push((
            "bad-policy-digest",
            bad_policy_digest,
            receipt.clone(),
            false,
        ));
        let mut typed_policy_digest = binding.clone();
        typed_policy_digest["policy_digest"] = json!(42);
        cases.push((
            "typed-policy-digest",
            typed_policy_digest,
            receipt.clone(),
            false,
        ));
        let mut bad_policy_source = binding.clone();
        bad_policy_source["policy_source"] = json!("legacy");
        cases.push((
            "bad-policy-source",
            bad_policy_source,
            receipt.clone(),
            false,
        ));
        let mut typed_policy_source = binding.clone();
        typed_policy_source["policy_source"] = json!(42);
        cases.push(("typed-policy-source", typed_policy_source, receipt, false));

        for (index, (name, binding, receipt, should_insert)) in cases.into_iter().enumerate() {
            let result = conn.execute(
                "INSERT INTO evidence_receipts(
                  id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
                  trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
                ) VALUES (
                  ?1, 'p-evidence', 'obl-1', 'attempt-1', 'trusted', ?2,
                  ?3, '[]', '{}', ?4, datetime('now')
                )",
                params![
                    format!("receipt-binding-{name}"),
                    format!("sha256:{index:064x}"),
                    binding.to_string(),
                    receipt.to_string(),
                ],
            );
            assert_eq!(
                result.is_ok(),
                should_insert,
                "{name} trigger expectation failed: {result:?}"
            );
        }
    }

    fn insert_attempt_with_result(
        conn: &Connection,
        id: &str,
        status: &str,
        completed_at: Option<&str>,
        exit_code: Option<i64>,
        attempt_json: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO evidence_attempts(
              id, project_id, obligation_id, capability_instance_id, attempt_status,
              execution_contract_digest, resolved_command_json, environment_digest,
              started_at, completed_at, exit_code, stdout_digest, stderr_digest,
              output_bounds_json, attempt_json, created_at
            ) VALUES (
              ?1, 'p-evidence', 'obl-1', 'cap-instance-1', ?2,
              ?3, '{\"cmd\":\"true\"}', ?4, '2026-07-28T12:00:00Z', ?5, ?6,
              ?7, ?7, '{}', ?8, datetime('now')
            )",
            params![
                id,
                status,
                DIGEST,
                DIGEST_B,
                completed_at,
                exit_code,
                DIGEST_C,
                attempt_json
            ],
        )
    }

    fn insert_retry_attempt(
        conn: &Connection,
        id: &str,
        retry_predecessor_attempt_id: Option<&str>,
        attempt_number: u32,
        max_attempts: u32,
        previous_attempt_ids: &[&str],
    ) -> rusqlite::Result<usize> {
        let attempt_json = json!({
            "status": "passed",
            "exit": {
                "exit_code": 0,
                "signal": null,
                "error": null
            },
            "retry_lineage": {
                "attempt_number": attempt_number,
                "max_attempts": max_attempts,
                "previous_attempt_ids": previous_attempt_ids
            }
        });
        conn.execute(
            "INSERT INTO evidence_attempts(
              id, project_id, obligation_id, capability_instance_id, attempt_status,
              execution_contract_digest, resolved_command_json, environment_digest,
              retry_predecessor_attempt_id, started_at, completed_at, exit_code,
              stdout_digest, stderr_digest, output_bounds_json, attempt_json, created_at
            ) VALUES (
              ?1, 'p-evidence', 'obl-1', 'cap-instance-1', 'passed',
              ?2, '{\"cmd\":\"true\"}', ?3, ?4,
              '2026-07-28T12:00:00Z', '2026-07-28T12:00:01Z', 0,
              ?5, ?5, '{}', ?6, datetime('now')
            )",
            params![
                id,
                DIGEST,
                DIGEST_B,
                retry_predecessor_attempt_id,
                DIGEST_C,
                serde_json::to_string(&attempt_json).unwrap()
            ],
        )
    }

    fn waiver_json(id: &str, scope_kind: &str, scope_id: &str, observation_ids: &[&str]) -> String {
        let observations = observation_ids
            .iter()
            .map(|observation_id| format!("\"{observation_id}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"id\":\"{id}\",\"schema_version\":\"evidence.contract.v1\",\"scope\":{{\"kind\":\"{scope_kind}\",\"id\":\"{scope_id}\"}},\"observation_ids\":[{observations}],\"source\":{{\"revision\":\"0123456789abcdef0123456789abcdef01234567\",\"tree_digest\":\"{DIGEST_C}\",\"dirty\":false}},\"target\":{{\"kind\":\"local_service\",\"uri\":\"http://127.0.0.1:4173/health\",\"digest\":\"{DIGEST_B}\"}},\"reason\":\"temporary outage\",\"created_by\":\"reviewer\",\"created_at\":\"{WAIVER_CREATED_AT}\",\"expires_at\":\"{WAIVER_EXPIRES_AT}\",\"approval_ref\":\"i-approval\",\"audit_trail\":[{{\"event\":\"created\",\"at\":\"{WAIVER_CREATED_AT}\"}}]}}"
        )
    }

    fn test_digest(seed: u8) -> String {
        format!("sha256:{seed:064x}")
    }

    fn assert_canonical_waiver_schema(waiver_json: &str, expected: bool) {
        let schema = serde_json::from_str::<serde_json::Value>(include_str!(
            "../../docs/contracts/fixtures/evidence/v1/schemas/evidence-contract-v1.schema.json"
        ))
        .unwrap();
        let validator = jsonschema::draft202012::options().build(&schema).unwrap();
        let value = serde_json::from_str::<serde_json::Value>(waiver_json).unwrap();
        let errors = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert_eq!(errors.is_empty(), expected, "{errors:?}");
    }

    fn assert_storage_waiver_contract(id: &str, waiver_json: &str, expected: bool) {
        let row = super::EvidenceWaiverContractRow {
            id,
            approval_ref: "i-approval",
            scope_kind: "item",
            scope_id: "i-target",
            reason: "temporary outage",
            created_by: "reviewer",
            expires_at: WAIVER_EXPIRES_AT,
            created_at: WAIVER_CREATED_AT,
        };
        assert_eq!(
            super::evidence_waiver_contract_is_valid(waiver_json, row),
            expected
        );
    }

    fn assert_validated_import_rejects_matching_digest(conn: &Connection, mut proposal: Value) {
        let id = proposal["id"].as_str().unwrap().to_string();
        let digest = crate::canonical_json::sha256_json_digest(&proposal).unwrap();
        let proposal_json = proposal.take().to_string();
        assert!(
            conn.execute(
                "INSERT INTO evidence_validated_imports(
                  project_id, id, digest, proposal_json, created_at
                ) VALUES ('p-evidence', ?1, ?2, ?3, datetime('now'))",
                params![id, digest, proposal_json],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_schema_creates_clean_install_and_is_idempotent() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();

        let evidence_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table'
                 AND name IN (
                   'proof_obligations',
                   'verification_capability_manifests',
                   'verification_capability_instances',
                   'evidence_attempts',
                   'evidence_attempt_artifacts',
                   'evidence_receipts',
                   'evidence_observation_results',
                   'coverage_verdicts',
                   'coverage_verdict_history',
                   'evidence_waivers',
                   'evidence_waiver_observations'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(evidence_tables, 11);
    }

    #[test]
    fn evidence_schema_upgrade_preserves_legacy_rows() {
        let conn = conn();
        conn.execute_batch(
            "CREATE TABLE projects(
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              root_path TEXT NOT NULL,
              status TEXT NOT NULL DEFAULT 'active',
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE logs(
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              item_id TEXT NOT NULL,
              kind TEXT NOT NULL,
              summary TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
            VALUES ('p-old', 'Old Project', '.', 'active', datetime('now'), datetime('now'));
            INSERT INTO logs(id, project_id, item_id, kind, summary, created_at)
            VALUES ('log-old', 'p-old', 'i-old', 'verification', 'legacy claim', datetime('now'));",
        )
        .unwrap();

        ensure_schema(&conn).unwrap();

        let project_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id = 'p-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_log_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM logs WHERE id = 'log-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let receipt_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM evidence_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(project_count, 1);
        assert_eq!(legacy_log_count, 1);
        assert_eq!(receipt_count, 0);
    }

    #[test]
    fn evidence_schema_upgrade_refreshes_attempt_result_trigger_for_unavailable_attempts() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        conn.execute_batch(
            "DROP TRIGGER evidence_attempts_status_result_invariant;
             CREATE TRIGGER evidence_attempts_status_result_invariant
             BEFORE INSERT ON evidence_attempts
             WHEN NEW.attempt_status = 'unavailable'
             BEGIN
               SELECT RAISE(ABORT, 'legacy unavailable attempts required process exit_code');
             END;",
        )
        .unwrap();

        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);

        let trigger_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type = 'trigger'
                   AND name = 'evidence_attempts_status_result_invariant'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(trigger_sql.contains("$.exit.exit_code"), "{trigger_sql}");
        assert!(
            trigger_sql.contains("julianday(NEW.completed_at) < julianday(NEW.started_at)"),
            "{trigger_sql}"
        );
        assert!(
            !trigger_sql.contains("legacy unavailable attempts"),
            "{trigger_sql}"
        );
        insert_attempt_with_result(
            &conn,
            "attempt-upgraded-unavailable",
            "unavailable",
            Some("2026-07-28T12:00:00.1001Z"),
            None,
            &attempt_json("unavailable", "null", "null", "\"unavailable\""),
        )
        .unwrap();
    }

    #[test]
    fn evidence_schema_upgrade_rejects_malformed_legacy_waivers() {
        let conn = conn();
        conn.execute_batch(
            "CREATE TABLE evidence_waivers(
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              approval_item_id TEXT NOT NULL,
              obligation_id TEXT,
              observation_id TEXT NOT NULL,
              scope_kind TEXT NOT NULL,
              scope_id TEXT NOT NULL,
              waiver_digest TEXT NOT NULL UNIQUE,
              reason TEXT NOT NULL,
              expires_at TEXT NOT NULL,
              created_by TEXT NOT NULL,
              waiver_json TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'legacy-empty-waiver', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'item', 'i-target',
              'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
              'temporary outage', '2026-08-01T00:00:00Z', 'reviewer',
              '{}', '2026-07-28T12:00:00Z'
            );",
        )
        .unwrap();

        let error = ensure_schema(&conn).unwrap_err().to_string();
        assert!(
            error.contains("malformed waiver contract for legacy-empty-waiver"),
            "{error}"
        );
    }

    #[test]
    fn evidence_schema_upgrade_backfills_legacy_capability_instance_manifest_digest() {
        let conn = conn();
        conn.execute_batch(
            "CREATE TABLE verification_capability_manifests(
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
              probe_execution_id TEXT NOT NULL,
              availability_status TEXT NOT NULL,
              runtime_target_json TEXT NOT NULL,
              host_fingerprint_json TEXT NOT NULL DEFAULT '{}',
              capability_snapshot_json TEXT NOT NULL,
              probe_result_json TEXT NOT NULL,
              created_at TEXT NOT NULL,
              valid_until TEXT
            );
            INSERT INTO verification_capability_manifests(
              id, version, adapter_kind, adapter_digest, manifest_digest, manifest_json, created_at
            ) VALUES ('legacy.capability', '1.0.0', 'process', 'adapter', 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', '{}', datetime('now'));
            INSERT INTO verification_capability_instances(
              id, manifest_id, manifest_version, probe_execution_id, availability_status,
              runtime_target_json, capability_snapshot_json, probe_result_json, created_at
            ) VALUES (
              'legacy-instance', 'legacy.capability', '1.0.0', 'legacy-probe', 'available',
              '{}', '{}', '{}', datetime('now')
            );",
        )
        .unwrap();

        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();

        let manifest_digest: String = conn
            .query_row(
                "SELECT manifest_digest FROM verification_capability_instances WHERE id = 'legacy-instance'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(manifest_digest, DIGEST_B);
        assert!(
            conn.execute(
                "UPDATE verification_capability_instances SET manifest_digest = ?1 WHERE id = 'legacy-instance'",
                params![DIGEST_C],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM verification_capability_instances WHERE id = 'legacy-instance'",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_attempts_and_receipts_are_immutable() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        insert_attempt(&conn);
        let (receipt_json, trusted_binding_json) = valid_receipt_and_binding_json();
        conn.execute(
            "INSERT INTO evidence_receipts(
              id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
              trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
            ) VALUES (
              'receipt-1', 'p-evidence', 'obl-1', 'attempt-1', 'trusted', ?1,
              ?2, '[]', '{}', ?3, datetime('now')
            )",
            params![DIGEST, trusted_binding_json, receipt_json],
        )
        .unwrap();

        assert!(
            conn.execute(
                "UPDATE evidence_attempts SET attempt_status = 'failed' WHERE id = 'attempt-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute("DELETE FROM evidence_receipts WHERE id = 'receipt-1'", [],)
                .is_err()
        );
        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, logical_name, path, digest, root_kind, created_at
            ) VALUES (
              'artifact-link-1', 'attempt-1', 'stdout', 'Cargo.toml', ?1, 'project', datetime('now')
            )",
            params![DIGEST_B],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO evidence_observation_results(
              id, receipt_id, observation_id, observation_type, subject,
              result_status, payload_json, created_at
            ) VALUES (
              'obs-1', 'receipt-1', 'http.ok', 'example.http.ok', 'http://localhost',
              'covered', '{}', datetime('now')
            )",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "UPDATE evidence_attempt_artifacts SET digest = ?1 WHERE id = 'artifact-link-1'",
                params![DIGEST_C],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM evidence_attempt_artifacts WHERE id = 'artifact-link-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE evidence_observation_results SET result_status = 'missing' WHERE id = 'obs-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM evidence_observation_results WHERE id = 'obs-1'",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_constraints_reject_conflicting_identities_and_escaped_paths() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        insert_attempt(&conn);

        assert!(
            conn.execute(
                "INSERT INTO evidence_attempts(
                  id, project_id, attempt_status, execution_contract_digest,
                  resolved_command_json, environment_digest, started_at, attempt_json, created_at
                ) VALUES (
                  'attempt-1', 'p-evidence', 'failed', ?1, '{}', ?2, datetime('now'), '{}', datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, logical_name, path, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-1', 'attempt-1', 'stdout', '../outside.log', ?1, 'project', datetime('now')
                )",
                params![DIGEST],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_validated_imports_require_project_digest_and_are_immutable() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        let proposal = json!({
            "id": "import-1",
            "schema_version": "evidence.contract.v1",
            "source_kind": "artifact_import",
            "submitted_at": "2026-07-28T12:00:00Z",
            "claims": {},
            "artifact_refs": [],
            "producer_metadata": {},
        });
        let proposal_json = proposal.to_string();
        let proposal_digest = crate::canonical_json::sha256_json_digest(&proposal).unwrap();

        assert!(
            conn.execute(
                "INSERT INTO evidence_validated_imports(
                  project_id, id, digest, proposal_json, created_at
                ) VALUES ('missing-project', 'import-1', ?1, ?2, datetime('now'))",
                params![proposal_digest, proposal_json],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_validated_imports(
                  project_id, id, digest, proposal_json, created_at
                ) VALUES ('p-evidence', 'import-bad-digest', 'not-a-digest', '{}', datetime('now'))",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_validated_imports(
                  project_id, id, digest, proposal_json, created_at
                ) VALUES ('p-evidence', 'import-1', ?1, '{}', datetime('now'))",
                params![proposal_digest],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_validated_imports(
                  project_id, id, digest, proposal_json, created_at
                ) VALUES ('p-evidence', 'wrong-id', ?1, ?2, datetime('now'))",
                params![proposal_digest, proposal_json],
            )
            .is_err()
        );
        for invalid_proposal in [
            {
                let mut invalid = proposal.clone();
                invalid["id"] = json!("import-invalid-schema-version");
                invalid["schema_version"] = json!("evidence.contract.v9");
                invalid
            },
            {
                let mut invalid = proposal.clone();
                invalid["id"] = json!("import-invalid-source-kind");
                invalid["source_kind"] = json!("trusted_host");
                invalid
            },
            {
                let mut invalid = proposal.clone();
                invalid["id"] = json!("import-invalid-timestamp");
                invalid["submitted_at"] = json!("not-a-timestamp");
                invalid
            },
            {
                let mut invalid = proposal.clone();
                invalid["id"] = json!("import-malformed-artifact-ref");
                invalid["artifact_refs"] =
                    json!([{"id": "artifact-bad", "kind": "stdout", "digest": "not-a-digest"}]);
                invalid
            },
            {
                let mut invalid = proposal.clone();
                invalid["id"] = json!("import-nested-authority");
                invalid["claims"] = json!({"nested": {"receipt_digest": DIGEST}});
                invalid
            },
            {
                let mut invalid = proposal.clone();
                invalid["id"] = json!("import-unknown-field");
                invalid["unknown_top_level"] = json!(true);
                invalid
            },
        ] {
            assert_validated_import_rejects_matching_digest(&conn, invalid_proposal);
        }

        conn.execute(
            "INSERT INTO evidence_validated_imports(
              project_id, id, digest, proposal_json, created_at
            ) VALUES ('p-evidence', 'import-1', ?1, ?2, datetime('now'))",
            params![proposal_digest, proposal_json],
        )
        .unwrap();

        assert!(
            conn.execute(
                "UPDATE evidence_validated_imports SET digest = ?1 WHERE project_id = 'p-evidence' AND id = 'import-1'",
                params![DIGEST_B],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM evidence_validated_imports WHERE project_id = 'p-evidence' AND id = 'import-1'",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_attempts_and_receipts_require_same_project_obligation_identity() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-other', 'Other', '.', 'active', datetime('now'), datetime('now'));
             INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
             VALUES ('pln-other', 'p-other', 'build', '/tmp/planr/other.plan.md', 'Other Plan', 'other-plan', 'ok', 'hash-other', datetime('now'), datetime('now'));
             INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
             VALUES ('i-other', 'p-other', 'Other item', 'Other item', 'ready', 'code', '/tmp/planr/other.plan.md', datetime('now'), datetime('now'));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proof_obligations(
              id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
              binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
              assurance_policy_json, policy_digest, config_digest, created_at
            ) VALUES (
              'obl-other', 'p-other', 'pln-other', 'i-other', 'crit-other', 1, 'Other obligation',
              1, '[{\"id\":\"obs-other\",\"type\":\"example.other\"}]', '{}', '{}', '{}', ?1, ?2, datetime('now')
            )",
            params![DIGEST, DIGEST_B],
        )
        .unwrap();

        assert!(
            conn.execute(
                "INSERT INTO evidence_attempts(
                  id, project_id, obligation_id, capability_instance_id, attempt_status,
                  execution_contract_digest, resolved_command_json, environment_digest,
                  started_at, completed_at, exit_code, attempt_json, created_at
                ) VALUES (
                  'attempt-cross-project', 'p-evidence', 'obl-other', 'cap-instance-1', 'passed',
                  ?1, '{}', ?2, datetime('now'), datetime('now'), 0, '{}', datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .is_err()
        );

        insert_attempt(&conn);
        assert!(
            conn.execute(
                "INSERT INTO evidence_receipts(
                  id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
                  trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
                ) VALUES (
                  'receipt-wrong-obligation', 'p-evidence', 'obl-other', 'attempt-1', 'trusted', ?1,
                  '{}', '[]', '{}', '{}', datetime('now')
                )",
                params![DIGEST],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_receipts(
                  id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
                  trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
                ) VALUES (
                  'receipt-wrong-project', 'p-other', 'obl-1', 'attempt-1', 'trusted', ?1,
                  '{}', '[]', '{}', '{}', datetime('now')
                )",
                params![DIGEST_B],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_attempts_enforce_status_result_sql_invariants() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);

        for (id, status, exit_code, attempt_json) in [
            (
                "attempt-valid-passed",
                "passed",
                Some(0),
                attempt_json("passed", "0", "null", "null"),
            ),
            (
                "attempt-valid-failed",
                "failed",
                Some(1),
                attempt_json("failed", "1", "null", "\"assertion_failed\""),
            ),
            (
                "attempt-valid-timed-out",
                "timed_out",
                None,
                attempt_json("timed_out", "null", "null", "\"timed_out\""),
            ),
            (
                "attempt-valid-skipped",
                "skipped",
                None,
                attempt_json("skipped", "null", "null", "\"skipped\""),
            ),
            (
                "attempt-valid-aborted",
                "aborted",
                None,
                attempt_json("aborted", "null", "\"SIGINT\"", "\"aborted\""),
            ),
            (
                "attempt-valid-unavailable",
                "unavailable",
                None,
                attempt_json("unavailable", "null", "null", "\"unavailable\""),
            ),
            (
                "attempt-valid-inconclusive",
                "inconclusive",
                Some(0),
                attempt_json("inconclusive", "0", "null", "\"inconclusive\""),
            ),
        ] {
            insert_attempt_with_result(
                &conn,
                id,
                status,
                Some("2026-07-28T12:00:01Z"),
                exit_code,
                &attempt_json,
            )
            .unwrap();
        }

        insert_attempt_with_result(
            &conn,
            "attempt-valid-variable-rfc3339-precision",
            "unavailable",
            Some("2026-07-28T12:00:00.1001Z"),
            None,
            &attempt_json("unavailable", "null", "null", "\"unavailable\""),
        )
        .expect("RFC3339 timestamps must be compared as instants, not lexicographic text");

        for (id, status, exit_code, attempt_json) in [
            (
                "attempt-bad-passed-exit",
                "passed",
                Some(1),
                attempt_json("passed", "1", "null", "null"),
            ),
            (
                "attempt-bad-failed-zero",
                "failed",
                Some(0),
                attempt_json("failed", "0", "null", "null"),
            ),
            (
                "attempt-bad-timed-out-exit",
                "timed_out",
                Some(124),
                attempt_json("timed_out", "124", "null", "\"timed_out\""),
            ),
            (
                "attempt-bad-skipped-exit",
                "skipped",
                Some(0),
                attempt_json("skipped", "0", "null", "\"skipped\""),
            ),
            (
                "attempt-bad-aborted-no-signal",
                "aborted",
                None,
                attempt_json("aborted", "null", "null", "\"aborted\""),
            ),
            (
                "attempt-bad-unavailable-exit",
                "unavailable",
                Some(0),
                attempt_json("unavailable", "0", "null", "\"unavailable\""),
            ),
            (
                "attempt-bad-inconclusive-marker",
                "inconclusive",
                Some(0),
                attempt_json("inconclusive", "0", "null", "null"),
            ),
            (
                "attempt-bad-status-mismatch",
                "passed",
                Some(0),
                attempt_json("failed", "0", "null", "null"),
            ),
        ] {
            let error = insert_attempt_with_result(
                &conn,
                id,
                status,
                Some("2026-07-28T12:00:01Z"),
                exit_code,
                &attempt_json,
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("evidence_attempts status must match execution result"),
                "{id}: {error}"
            );
        }

        let error = insert_attempt_with_result(
            &conn,
            "attempt-bad-timing",
            "passed",
            Some("2026-07-28T11:59:59Z"),
            Some(0),
            &attempt_json("passed", "0", "null", "null"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("evidence_attempts status must match execution result"),
            "{error}"
        );
    }

    #[test]
    fn evidence_attempts_enforce_exact_retry_cap_and_history_prefix() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);

        insert_retry_attempt(&conn, "attempt-lineage-1", None, 1, 3, &[]).unwrap();
        insert_retry_attempt(
            &conn,
            "attempt-lineage-2",
            Some("attempt-lineage-1"),
            2,
            3,
            &["attempt-lineage-1"],
        )
        .unwrap();
        insert_retry_attempt(
            &conn,
            "attempt-lineage-3",
            Some("attempt-lineage-2"),
            3,
            3,
            &["attempt-lineage-1", "attempt-lineage-2"],
        )
        .unwrap();

        let cap_drift_error = insert_retry_attempt(
            &conn,
            "attempt-cap-drift",
            Some("attempt-lineage-2"),
            3,
            4,
            &["attempt-lineage-1", "attempt-lineage-2"],
        )
        .unwrap_err()
        .to_string();
        assert!(
            cap_drift_error.contains(
                "evidence_attempts retry lineage must reference the prior same-scope attempt"
            ),
            "{cap_drift_error}"
        );

        let forged_prefix_error = insert_retry_attempt(
            &conn,
            "attempt-forged-prefix",
            Some("attempt-lineage-2"),
            3,
            3,
            &["attempt-forged-parent", "attempt-lineage-2"],
        )
        .unwrap_err()
        .to_string();
        assert!(
            forged_prefix_error.contains(
                "evidence_attempts retry lineage must reference the prior same-scope attempt"
            ),
            "{forged_prefix_error}"
        );

        let exhausted_cap_error = insert_retry_attempt(
            &conn,
            "attempt-exhausted-cap",
            Some("attempt-lineage-3"),
            4,
            3,
            &[
                "attempt-lineage-1",
                "attempt-lineage-2",
                "attempt-lineage-3",
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(
            exhausted_cap_error.contains(
                "evidence_attempts retry lineage must reference the prior same-scope attempt"
            ),
            "{exhausted_cap_error}"
        );
    }

    fn insert_artifact_link(
        conn: &Connection,
        id: &str,
        logical_name: &str,
        path: &str,
        root_kind: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, logical_name, path, digest, root_kind, created_at
            ) VALUES (
              ?1, 'attempt-1', ?2, ?3, ?4, ?5, datetime('now')
            )",
            params![id, logical_name, path, DIGEST, root_kind],
        )
    }

    #[test]
    fn evidence_artifact_paths_enforce_selected_roots_and_reject_external_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".planr/artifacts")).unwrap();
        fs::write(root.path().join("project.log"), "project").unwrap();
        fs::write(root.path().join(".planr/planr.log"), "planr").unwrap();
        fs::write(root.path().join(".planr/artifacts/store.log"), "store").unwrap();

        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item_with_root(&conn, root.path().to_str().unwrap());
        seed_binding(&conn);
        insert_attempt(&conn);

        insert_artifact_link(
            &conn,
            "artifact-project",
            "project",
            "project.log",
            "project",
        )
        .unwrap();
        insert_artifact_link(&conn, "artifact-planr", "planr", "planr.log", "planr").unwrap();
        insert_artifact_link(
            &conn,
            "artifact-store",
            "store",
            "store.log",
            "artifact_store",
        )
        .unwrap();

        assert!(
            insert_artifact_link(
                &conn,
                "artifact-wrong-planr-root",
                "wrong-planr",
                "project.log",
                "planr",
            )
            .is_err()
        );
        assert!(
            insert_artifact_link(
                &conn,
                "artifact-wrong-store-root",
                "wrong-store",
                ".planr/artifacts/store.log",
                "artifact_store",
            )
            .is_err()
        );
        assert!(
            insert_artifact_link(
                &conn,
                "artifact-external",
                "external",
                "https://example.invalid/evidence.log",
                "external",
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_artifact_links_require_concrete_same_project_contained_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("inside.log"), "inside").unwrap();
        fs::write(root.path().join("other.log"), "other").unwrap();

        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item_with_root(&conn, root.path().to_str().unwrap());
        seed_binding(&conn);
        insert_attempt(&conn);
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-other', 'Other', '.', 'active', datetime('now'), datetime('now'));
             INSERT INTO items(id, project_id, title, description, status, work_type, created_at, updated_at)
             VALUES ('i-other', 'p-other', 'Other item', 'Other item', 'ready', 'code', datetime('now'), datetime('now'));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts(id, project_id, item_id, name, path, created_at)
             VALUES ('stored-inside', 'p-evidence', 'i-approval', 'inside', 'inside.log', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts(id, project_id, item_id, name, path, created_at)
             VALUES ('stored-cross-project', 'p-other', 'i-other', 'cross', 'inside.log', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts(id, project_id, item_id, name, path, created_at)
             VALUES ('stored-escaped', 'p-evidence', 'i-approval', 'escaped', '../outside.log', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts(id, project_id, item_id, name, path, created_at)
             VALUES ('stored-no-path', 'p-evidence', 'i-approval', 'no-path', NULL, datetime('now'))",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, logical_name, path, digest, root_kind, created_at
            ) VALUES (
              'artifact-link-path-only', 'attempt-1', 'path-only', 'inside.log',
              ?1, 'project', datetime('now')
            )",
            params![DIGEST],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, artifact_id, logical_name, digest, root_kind, created_at
            ) VALUES (
              'artifact-link-stored-inside', 'attempt-1', 'stored-inside', 'stored-inside',
              ?1, 'project', datetime('now')
            )",
            params![DIGEST],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, artifact_id, logical_name, path, digest, root_kind, created_at
            ) VALUES (
              'artifact-link-stored-inside-with-path', 'attempt-1', 'stored-inside',
              'stored-inside-with-path', 'inside.log', ?1, 'project', datetime('now')
            )",
            params![DIGEST],
        )
        .unwrap();

        let null_error = conn
            .execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, logical_name, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-null', 'attempt-1', 'null-link', ?1, 'project', datetime('now')
                )",
                params![DIGEST_B],
            )
            .unwrap_err()
            .to_string();
        assert!(
            null_error.contains("evidence_attempt_artifacts require path or artifact_id"),
            "{null_error}"
        );

        let cross_project_error = conn
            .execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, artifact_id, logical_name, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-cross-project', 'attempt-1', 'stored-cross-project',
                  'cross-project', ?1, 'project', datetime('now')
                )",
                params![DIGEST_B],
            )
            .unwrap_err()
            .to_string();
        assert!(
            cross_project_error.contains(
                "evidence_attempt_artifacts artifact_id must belong to attempt project_id"
            ),
            "{cross_project_error}"
        );

        let escaped_artifact_error = conn
            .execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, artifact_id, logical_name, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-escaped', 'attempt-1', 'stored-escaped', 'escaped',
                  ?1, 'project', datetime('now')
                )",
                params![DIGEST_C],
            )
            .unwrap_err()
            .to_string();
        assert!(
            escaped_artifact_error
                .contains("evidence_attempt_artifacts path must stay inside its root"),
            "{escaped_artifact_error}"
        );

        let mismatched_path_error = conn
            .execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, artifact_id, logical_name, path, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-path-mismatch', 'attempt-1', 'stored-inside',
                  'path-mismatch', 'other.log', ?1, 'project', datetime('now')
                )",
                params![DIGEST],
            )
            .unwrap_err()
            .to_string();
        assert!(
            mismatched_path_error
                .contains("evidence_attempt_artifacts artifact_id path must match supplied path"),
            "{mismatched_path_error}"
        );

        let escaped_artifact_with_safe_path_error = conn
            .execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, artifact_id, logical_name, path, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-escaped-with-safe-path', 'attempt-1', 'stored-escaped',
                  'escaped-with-safe-path', 'inside.log', ?1, 'project', datetime('now')
                )",
                params![DIGEST],
            )
            .unwrap_err()
            .to_string();
        assert!(
            escaped_artifact_with_safe_path_error
                .contains("evidence_attempt_artifacts artifact_id path must match supplied path"),
            "{escaped_artifact_with_safe_path_error}"
        );

        let missing_stored_path_error = conn
            .execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, artifact_id, logical_name, path, digest, root_kind, created_at
                ) VALUES (
                  'artifact-link-no-stored-path', 'attempt-1', 'stored-no-path',
                  'no-stored-path', 'inside.log', ?1, 'project', datetime('now')
                )",
                params![DIGEST],
            )
            .unwrap_err()
            .to_string();
        assert!(
            missing_stored_path_error
                .contains("evidence_attempt_artifacts artifact_id path must match supplied path"),
            "{missing_stored_path_error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn evidence_artifact_paths_reject_symlink_escapes() {
        use std::{fs, os::unix::fs::symlink};

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.log");
        fs::write(&outside_file, "outside").unwrap();
        fs::write(root.path().join("inside.log"), "inside").unwrap();
        symlink(&outside_file, root.path().join("escape.log")).unwrap();

        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item_with_root(&conn, root.path().to_str().unwrap());
        seed_binding(&conn);
        insert_attempt(&conn);

        conn.execute(
            "INSERT INTO evidence_attempt_artifacts(
              id, attempt_id, logical_name, path, digest, root_kind, created_at
            ) VALUES (
              'artifact-contained', 'attempt-1', 'inside', 'inside.log', ?1, 'project', datetime('now')
            )",
            params![DIGEST],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO evidence_attempt_artifacts(
                  id, attempt_id, logical_name, path, digest, root_kind, created_at
                ) VALUES (
                  'artifact-escape', 'attempt-1', 'escape', 'escape.log', ?1, 'project', datetime('now')
                )",
                params![DIGEST_B],
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reopened_connections_register_artifact_path_validator_without_schema_setup() {
        use std::{fs, os::unix::fs::symlink};

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let db_path = root.path().join(".planr/reopen.sqlite");
        let outside_file = outside.path().join("outside.log");
        fs::write(&outside_file, "outside").unwrap();
        fs::write(root.path().join("inside.log"), "inside").unwrap();
        symlink(&outside_file, root.path().join("escape.log")).unwrap();

        {
            let conn = open_db(&db_path).unwrap();
            ensure_schema(&conn).unwrap();
            seed_project_and_item_with_root(&conn, root.path().to_str().unwrap());
            seed_binding(&conn);
            insert_attempt(&conn);
        }

        let conn = open_db(&db_path).unwrap();
        insert_artifact_link(
            &conn,
            "artifact-contained-reopened",
            "contained",
            "inside.log",
            "project",
        )
        .unwrap();
        let error = insert_artifact_link(
            &conn,
            "artifact-escape-reopened",
            "escape",
            "escape.log",
            "project",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("evidence_attempt_artifacts path must stay inside its root"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn evidence_artifact_paths_reject_missing_paths_that_could_later_escape() {
        use std::{fs, os::unix::fs::symlink};

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_target = outside.path().join("future.log");
        symlink(&outside_target, root.path().join("broken-link.log")).unwrap();

        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item_with_root(&conn, root.path().to_str().unwrap());
        seed_binding(&conn);
        insert_attempt(&conn);

        assert!(
            insert_artifact_link(
                &conn,
                "artifact-broken-link",
                "broken-link",
                "broken-link.log",
                "project",
            )
            .is_err()
        );

        assert!(
            insert_artifact_link(
                &conn,
                "artifact-missing-leaf",
                "missing-leaf",
                "future/missing.log",
                "project",
            )
            .is_err()
        );

        fs::create_dir(root.path().join("future")).unwrap();
        symlink(outside.path(), root.path().join("future/link-to-outside")).unwrap();
        assert!(
            insert_artifact_link(
                &conn,
                "artifact-later-symlink",
                "later-symlink",
                "future/link-to-outside/future.log",
                "project",
            )
            .is_err()
        );
    }

    #[test]
    fn proof_obligations_and_coverage_history_are_append_only_and_identity_bound() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        conn.execute(
            "INSERT INTO coverage_verdicts(
              id, project_id, scope_kind, scope_id, binding_digest, policy_digest,
              config_digest, coverage_status, verdict_json, computed_at
            ) VALUES (
              'verdict-1', 'p-evidence', 'item', 'i-approval', ?1, ?2, ?3,
              'satisfied', '{}', datetime('now')
            )",
            params![DIGEST, DIGEST_B, DIGEST_C],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO coverage_verdict_history(
              id, verdict_id, project_id, scope_kind, scope_id, binding_digest,
              policy_digest, config_digest, coverage_status, verdict_json, computed_at
            ) VALUES (
              'history-1', 'verdict-1', 'p-evidence', 'item', 'i-approval',
              ?1, ?2, ?3, 'satisfied', '{}', datetime('now')
            )",
            params![DIGEST, DIGEST_B, DIGEST_C],
        )
        .unwrap();

        assert!(
            conn.execute(
                "UPDATE proof_obligations SET title = 'rewritten' WHERE id = 'obl-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute("DELETE FROM proof_obligations WHERE id = 'obl-1'", [])
                .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO coverage_verdict_history(
                  id, verdict_id, project_id, scope_kind, scope_id, binding_digest,
                  policy_digest, config_digest, coverage_status, verdict_json, computed_at
                ) VALUES (
                  'history-mismatch', 'verdict-1', 'p-evidence', 'item', 'i-other',
                  ?1, ?2, ?3, 'satisfied', '{}', datetime('now')
                )",
                params![DIGEST, DIGEST_B, DIGEST_C],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE coverage_verdict_history SET coverage_status = 'stale' WHERE id = 'history-1'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM coverage_verdict_history WHERE id = 'history-1'",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn proof_obligations_require_same_project_plan_and_item_containment() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-foreign', 'Foreign', '.', 'active', datetime('now'), datetime('now'));
             INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
             VALUES ('pln-foreign', 'p-foreign', 'build', '/tmp/planr/evidence.plan.md', 'Foreign Plan', 'foreign-plan', 'ok', 'hash-foreign', datetime('now'), datetime('now'));
             INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
             VALUES ('i-foreign', 'p-foreign', 'Foreign', 'Foreign item', 'ready', 'code', '/tmp/planr/evidence.plan.md', datetime('now'), datetime('now'));",
        )
        .unwrap();

        assert!(
            conn.execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, policy_digest, config_digest, created_at
                ) VALUES (
                  'obl-foreign-plan', 'p-evidence', 'pln-foreign', NULL, 'crit-foreign-plan', 1, 'Foreign plan',
                  1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, policy_digest, config_digest, created_at
                ) VALUES (
                  'obl-foreign-item', 'p-evidence', 'pln-evidence', 'i-foreign', 'crit-foreign-item', 1, 'Foreign item',
                  1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .is_err()
        );
    }

    #[test]
    fn proof_obligation_containment_survives_reopen_schema_upgrade_path() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("planr.sqlite");
        {
            let conn = open_db(&db).unwrap();
            ensure_schema(&conn).unwrap();
            seed_project_and_item(&conn);
            seed_binding(&conn);
        }
        {
            let conn = open_db(&db).unwrap();
            ensure_schema(&conn).unwrap();
            conn.execute_batch(
                "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
                 VALUES ('p-reopen-foreign', 'Reopen Foreign', '.', 'active', datetime('now'), datetime('now'));
                 INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
                 VALUES ('pln-reopen-foreign', 'p-reopen-foreign', 'build', '/tmp/planr/evidence.plan.md', 'Reopen Foreign', 'reopen-foreign', 'ok', 'hash-reopen', datetime('now'), datetime('now'));
                 INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('i-reopen-foreign', 'p-reopen-foreign', 'Reopen Foreign', 'Foreign item', 'ready', 'code', '/tmp/planr/evidence.plan.md', datetime('now'), datetime('now'));",
            )
            .unwrap();
            assert!(
                conn.execute(
                    "INSERT INTO proof_obligations(
                      id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                      binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                      assurance_policy_json, policy_digest, config_digest, created_at
                    ) VALUES (
                      'obl-reopen-foreign-item', 'p-evidence', 'pln-evidence', 'i-reopen-foreign', 'crit-reopen-foreign-item', 1, 'Foreign item',
                      1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                    )",
                    params![DIGEST, DIGEST_B],
                )
                .is_err()
            );
            assert_current_proof_obligation_runtime_schema(&conn);
            conn.execute_batch(
                "DROP INDEX proof_obligations_item_idx;
                 DROP INDEX proof_obligations_project_identity_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(project_id, item_id);
                 CREATE UNIQUE INDEX proof_obligations_project_identity_idx
                   ON proof_obligations(project_id, plan_id, criterion_id, obligation_version);",
            )
            .unwrap();
            assert_eq!(
                proof_obligation_index_columns(&conn, "proof_obligations_item_idx"),
                vec!["project_id", "item_id"]
            );
            assert_eq!(
                proof_obligation_index_columns(&conn, "proof_obligations_project_identity_idx"),
                vec![
                    "project_id",
                    "plan_id",
                    "criterion_id",
                    "obligation_version"
                ]
            );
            ensure_schema(&conn).unwrap();
            assert_current_proof_obligation_runtime_schema(&conn);
            ensure_schema(&conn).unwrap();
            assert_current_proof_obligation_runtime_schema(&conn);
            for drift in [
                "DROP INDEX proof_obligations_item_idx;
                 CREATE UNIQUE INDEX proof_obligations_item_idx ON proof_obligations(item_id);",
                "DROP INDEX proof_obligations_project_identity_idx;
                 CREATE INDEX proof_obligations_project_identity_idx ON proof_obligations(id, project_id);",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id) WHERE item_id IS NOT NULL;",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id COLLATE NOCASE);",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id DESC);",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id COLLATE NOCASE DESC);",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(lower(item_id));",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(item_id) WHERE item_id IS NOT NULL AND project_id IS NOT NULL;",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON proof_obligations(project_id);",
                "DROP INDEX proof_obligations_project_identity_idx;
                 CREATE UNIQUE INDEX proof_obligations_project_identity_idx ON proof_obligations(project_id, id);",
                "DROP INDEX proof_obligations_item_idx;
                 CREATE INDEX proof_obligations_item_idx ON projects(name);",
            ] {
                conn.execute_batch(drift).unwrap();
                assert_ne!(
                    proof_obligation_runtime_schema(&conn),
                    current_proof_obligation_runtime_schema()
                );
                ensure_schema(&conn).unwrap();
                assert_current_proof_obligation_runtime_schema(&conn);
            }
        }
    }

    #[test]
    fn evidence_schema_upgrade_blocks_invalid_historical_proof_obligations_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("planr.sqlite");
        {
            let conn = open_db(&db).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE projects(
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   root_path TEXT NOT NULL,
                   description TEXT,
                   status TEXT NOT NULL DEFAULT 'active',
                   metadata TEXT,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 CREATE TABLE plans(
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   stage TEXT NOT NULL,
                   path TEXT NOT NULL,
                   title TEXT NOT NULL,
                   slug TEXT NOT NULL,
                   package_manifest TEXT,
                   frontmatter TEXT,
                   parse_status TEXT NOT NULL,
                   content_hash TEXT NOT NULL,
                   archived INTEGER NOT NULL DEFAULT 0,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 CREATE TABLE items(
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   parent_item_id TEXT,
                   title TEXT NOT NULL,
                   description TEXT NOT NULL,
                   status TEXT NOT NULL DEFAULT 'pending',
                   work_type TEXT NOT NULL DEFAULT 'generic',
                   priority INTEGER NOT NULL DEFAULT 0,
                   worker_id TEXT,
                   plan_path TEXT,
                   pick_token TEXT,
                   picked_at TEXT,
                   last_heartbeat_at TEXT,
                   progress_percent INTEGER,
                   progress_note TEXT,
                   paused_at TEXT,
                   timeout_seconds INTEGER,
                   max_retries INTEGER NOT NULL DEFAULT 0,
                   retry_count INTEGER NOT NULL DEFAULT 0,
                   retry_backoff TEXT NOT NULL DEFAULT 'exponential',
                   retry_delay_ms INTEGER NOT NULL DEFAULT 1000,
                   pre_condition TEXT,
                   post_condition TEXT,
                   approval_status TEXT,
                   approval_requested_at TEXT,
                   approved_by TEXT,
                   approval_comment TEXT,
                   started_at TEXT,
                   completed_at TEXT,
                   result TEXT,
                   error TEXT,
                   metadata TEXT,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 {PRIOR_PROOF_OBLIGATIONS_RUNTIME_DDL}
                 INSERT INTO meta(key, value) VALUES ('schema_version', '1');
                 INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
                 VALUES ('p-evidence', 'Evidence', '.', 'active', datetime('now'), datetime('now')),
                        ('p-foreign', 'Foreign', '.', 'active', datetime('now'), datetime('now'));
                 INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
                 VALUES ('pln-evidence', 'p-evidence', 'build', '/tmp/planr/evidence.plan.md', 'Evidence Plan', 'evidence-plan', 'ok', 'hash-evidence', datetime('now'), datetime('now'));
                 INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('i-valid', 'p-evidence', 'Valid', 'Valid item', 'closed', 'code', '/tmp/planr/evidence.plan.md', datetime('now'), datetime('now')),
                        ('i-foreign', 'p-foreign', 'Foreign', 'Foreign item', 'closed', 'code', '/tmp/planr/evidence.plan.md', datetime('now'), datetime('now'));"
            ))
            .unwrap();
            conn.execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, policy_digest, config_digest, created_at
                ) VALUES (
                  'obl-valid-history', 'p-evidence', 'pln-evidence', 'i-valid', 'crit-valid', 1, 'Valid history',
                  1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, policy_digest, config_digest, created_at
                ) VALUES (
                  'obl-invalid-history', 'p-evidence', 'pln-evidence', 'i-foreign', 'crit-invalid', 1, 'Invalid history',
                  1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .unwrap();
        }
        let before_schema;
        let before_rows;
        let before_version;
        {
            let conn = open_db(&db).unwrap();
            before_schema = sqlite_master_snapshot(&conn);
            before_rows = proof_obligation_row_snapshot(&conn);
            before_version = schema_version_value(&conn);
        }
        {
            let conn = open_db(&db).unwrap();
            let error = ensure_schema(&conn).unwrap_err().to_string();
            assert!(
                error.contains("proof_obligations containment upgrade blocked"),
                "{error}"
            );
            let trigger_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'trigger'
                       AND name IN ('proof_obligations_match_plan_project', 'proof_obligations_match_item_project_plan')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(trigger_count, 0);
            assert_eq!(sqlite_master_snapshot(&conn), before_schema);
            assert_eq!(proof_obligation_row_snapshot(&conn), before_rows);
            assert_eq!(schema_version_value(&conn), before_version);
            assert!(
                conn.execute(
                    "UPDATE proof_obligations SET title = 'mutated' WHERE id = 'obl-valid-history'",
                    [],
                )
                .is_err()
            );
            assert!(
                conn.execute(
                    "DELETE FROM proof_obligations WHERE id = 'obl-invalid-history'",
                    [],
                )
                .is_err()
            );
        }
        {
            let conn = open_db(&db).unwrap();
            let error = ensure_schema(&conn).unwrap_err().to_string();
            assert!(
                error.contains("proof_obligations containment upgrade blocked"),
                "{error}"
            );
            assert_eq!(sqlite_master_snapshot(&conn), before_schema);
            assert_eq!(proof_obligation_row_snapshot(&conn), before_rows);
            assert_eq!(schema_version_value(&conn), before_version);
        }

        let valid_dir = tempfile::tempdir().unwrap();
        let valid_db = valid_dir.path().join("planr.sqlite");
        let valid_rows_after_upgrade;
        let valid_version_after_upgrade;
        {
            let conn = open_db(&valid_db).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE projects(
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   root_path TEXT NOT NULL,
                   description TEXT,
                   status TEXT NOT NULL DEFAULT 'active',
                   metadata TEXT,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 CREATE TABLE plans(
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   stage TEXT NOT NULL,
                   path TEXT NOT NULL,
                   title TEXT NOT NULL,
                   slug TEXT NOT NULL,
                   package_manifest TEXT,
                   frontmatter TEXT,
                   parse_status TEXT NOT NULL,
                   content_hash TEXT NOT NULL,
                   archived INTEGER NOT NULL DEFAULT 0,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 CREATE TABLE items(
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   parent_item_id TEXT,
                   title TEXT NOT NULL,
                   description TEXT NOT NULL,
                   status TEXT NOT NULL DEFAULT 'pending',
                   work_type TEXT NOT NULL DEFAULT 'generic',
                   priority INTEGER NOT NULL DEFAULT 0,
                   worker_id TEXT,
                   plan_path TEXT,
                   pick_token TEXT,
                   picked_at TEXT,
                   last_heartbeat_at TEXT,
                   progress_percent INTEGER,
                   progress_note TEXT,
                   paused_at TEXT,
                   timeout_seconds INTEGER,
                   max_retries INTEGER NOT NULL DEFAULT 0,
                   retry_count INTEGER NOT NULL DEFAULT 0,
                   retry_backoff TEXT NOT NULL DEFAULT 'exponential',
                   retry_delay_ms INTEGER NOT NULL DEFAULT 1000,
                   pre_condition TEXT,
                   post_condition TEXT,
                   approval_status TEXT,
                   approval_requested_at TEXT,
                   approved_by TEXT,
                   approval_comment TEXT,
                   started_at TEXT,
                   completed_at TEXT,
                   result TEXT,
                   error TEXT,
                   metadata TEXT,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                 {PRIOR_PROOF_OBLIGATIONS_RUNTIME_DDL}
                 INSERT INTO meta(key, value) VALUES ('schema_version', '1');
                 INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
                 VALUES ('p-evidence', 'Evidence', '.', 'active', datetime('now'), datetime('now'));
                 INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, created_at, updated_at)
                 VALUES ('pln-evidence', 'p-evidence', 'build', '/tmp/planr/evidence.plan.md', 'Evidence Plan', 'evidence-plan', 'ok', 'hash-evidence', datetime('now'), datetime('now'));
                 INSERT INTO items(id, project_id, title, description, status, work_type, plan_path, created_at, updated_at)
                 VALUES ('i-valid', 'p-evidence', 'Valid', 'Valid item', 'closed', 'code', '/tmp/planr/evidence.plan.md', datetime('now'), datetime('now'));"
            ))
            .unwrap();
            conn.execute(
                "INSERT INTO proof_obligations(
                  id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                  binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                  assurance_policy_json, policy_digest, config_digest, created_at
                ) VALUES (
                  'obl-valid-history', 'p-evidence', 'pln-evidence', 'i-valid', 'crit-valid', 1, 'Valid history',
                  1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .unwrap();
            ensure_schema(&conn).unwrap();
            ensure_schema(&conn).unwrap();
            assert_current_proof_obligation_runtime_schema(&conn);
            let valid_survived: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM proof_obligations WHERE id = 'obl-valid-history'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(valid_survived, 1);
            let trigger_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'trigger'
                       AND name IN (
                         'proof_obligations_no_update',
                         'proof_obligations_no_delete',
                         'proof_obligations_match_plan_project',
                         'proof_obligations_match_item_project_plan'
                       )",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(trigger_count, 4);
            assert!(
                conn.execute(
                    "INSERT INTO proof_obligations(
                      id, project_id, plan_id, item_id, criterion_id, obligation_version, title,
                      binding, observation_requirements_json, fixture_policy_json, freshness_policy_json,
                      assurance_policy_json, policy_digest, config_digest, created_at
                    ) VALUES (
                      'obl-invalid-after-upgrade', 'p-evidence', 'pln-evidence', 'missing-item', 'crit-invalid-after', 1, 'Invalid after',
                      1, '[]', '{}', '{}', '{}', ?1, ?2, datetime('now')
                    )",
                    params![DIGEST, DIGEST_B],
                )
                .is_err()
            );
            valid_rows_after_upgrade = proof_obligation_row_snapshot(&conn);
            valid_version_after_upgrade = schema_version_value(&conn);
        }
        {
            let conn = open_db(&valid_db).unwrap();
            ensure_schema(&conn).unwrap();
            assert_current_proof_obligation_runtime_schema(&conn);
            assert_eq!(
                proof_obligation_row_snapshot(&conn),
                valid_rows_after_upgrade
            );
            assert_eq!(schema_version_value(&conn), valid_version_after_upgrade);
        }
        {
            let conn = open_db(&valid_db).unwrap();
            ensure_schema(&conn).unwrap();
            ensure_schema(&conn).unwrap();
            assert_current_proof_obligation_runtime_schema(&conn);
            assert_eq!(
                proof_obligation_row_snapshot(&conn),
                valid_rows_after_upgrade
            );
            assert_eq!(schema_version_value(&conn), valid_version_after_upgrade);
        }
    }

    #[test]
    fn evidence_waivers_must_link_existing_approval_items() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");
        let first_waiver_json = waiver_json("waiver-1", "item", "i-target", &["obs-http-200"]);

        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'waiver-1', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'item', 'i-target', ?1, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
            )",
            params![DIGEST, first_waiver_json],
        )
        .unwrap();
        let missing_approval_json = waiver_json("waiver-2", "item", "i-target", &["obs-http-200"]);
        assert!(
            conn.execute(
                "INSERT INTO evidence_waivers(
                  id, project_id, approval_item_id, obligation_id, observation_id,
                  scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                  waiver_json, created_at
                ) VALUES (
                  'waiver-2', 'p-evidence', 'missing-item', 'obl-1', 'obs-http-200',
                  'item', 'i-target', ?1, 'temporary outage',
                  '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
                )",
                params![DIGEST_B, missing_approval_json],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_waivers_require_same_project_approved_items() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        conn.execute_batch(
            "INSERT INTO projects(id, name, root_path, status, created_at, updated_at)
             VALUES ('p-other', 'Other', '.', 'active', datetime('now'), datetime('now'));
             INSERT INTO items(id, project_id, title, description, status, work_type, approval_status, created_at, updated_at)
             VALUES
               ('i-requested', 'p-evidence', 'Requested', 'Approval item', 'ready', 'code', 'requested', datetime('now'), datetime('now')),
               ('i-denied', 'p-evidence', 'Denied', 'Approval item', 'ready', 'code', 'denied', datetime('now'), datetime('now')),
               ('i-other-approved', 'p-other', 'Other approval', 'Approval item', 'ready', 'code', 'approved', datetime('now'), datetime('now'));",
        )
        .unwrap();

        for (idx, approval_item_id) in ["i-approval", "i-requested", "i-denied", "i-other-approved"]
            .into_iter()
            .enumerate()
        {
            assert!(
                conn.execute(
                    "INSERT INTO evidence_waivers(
                      id, project_id, approval_item_id, obligation_id, observation_id,
                      scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                      waiver_json, created_at
                    ) VALUES (
                      ?1, 'p-evidence', ?2, 'obl-1', 'obs-http-200',
                      'item', 'i-target', ?3, 'temporary outage',
                      '2026-08-01T00:00:00Z', 'reviewer', ?4, '2026-07-28T12:00:00Z'
                    )",
                    params![
                        format!("waiver-rejected-{idx}"),
                        approval_item_id,
                        DIGEST_C,
                        waiver_json(
                            &format!("waiver-rejected-{idx}"),
                            "item",
                            "i-target",
                            &["obs-http-200"]
                        )
                    ],
                )
                .is_err(),
                "{approval_item_id} should not authorize a waiver"
            );
        }
    }

    #[test]
    fn evidence_waivers_require_exact_observation_scope() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");

        for (id, obligation_id, observation_id, scope_kind, scope_id) in [
            (
                "waiver-missing-obligation",
                None,
                Some("obs-http-200"),
                "item",
                "i-target",
            ),
            (
                "waiver-missing-observation",
                Some("obl-1"),
                None,
                "item",
                "i-target",
            ),
            (
                "waiver-mismatched-observation",
                Some("obl-1"),
                Some("obs-not-required"),
                "item",
                "i-target",
            ),
            (
                "waiver-mismatched-observation-scope",
                Some("obl-1"),
                Some("obs-http-200"),
                "observation",
                "obs-other",
            ),
            (
                "waiver-unsupported-project-scope",
                Some("obl-1"),
                Some("obs-http-200"),
                "project",
                "p-evidence",
            ),
            (
                "waiver-unsupported-contract-scope",
                Some("obl-1"),
                Some("obs-http-200"),
                "obligation",
                "obl-1",
            ),
        ] {
            let observation_ids = observation_id.into_iter().collect::<Vec<_>>();
            assert!(
                conn.execute(
                    "INSERT INTO evidence_waivers(
                      id, project_id, approval_item_id, obligation_id, observation_id,
                      scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                      waiver_json, created_at
                    ) VALUES (
                      ?1, 'p-evidence', 'i-approval', ?2, ?3,
                      ?4, ?5, ?6, 'temporary outage',
                      '2026-08-01T00:00:00Z', 'reviewer', ?7, '2026-07-28T12:00:00Z'
                    )",
                    params![
                        id,
                        obligation_id,
                        observation_id,
                        scope_kind,
                        scope_id,
                        DIGEST_C,
                        waiver_json(id, scope_kind, scope_id, &observation_ids)
                    ],
                )
                .is_err(),
                "{id} should not authorize a waiver"
            );
        }

        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'waiver-observation-1', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'item', 'i-target', ?1, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
            )",
            params![
                DIGEST,
                waiver_json(
                    "waiver-observation-1",
                    "item",
                    "i-target",
                    &["obs-http-200"]
                )
            ],
        )
        .unwrap();

        assert!(
            conn.execute(
                "UPDATE evidence_waivers SET observation_id = 'obs-not-required' WHERE id = 'waiver-observation-1'",
                [],
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_waivers_persist_every_canonical_scope_kind() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");

        for (idx, (scope_kind, scope_id)) in [
            ("criterion", "crit-1"),
            ("item", "i-target"),
            ("plan", "pln-evidence"),
            ("goal", "goal-release"),
        ]
        .into_iter()
        .enumerate()
        {
            let waiver_id = format!("waiver-{scope_kind}");
            conn.execute(
                "INSERT INTO evidence_waivers(
                  id, project_id, approval_item_id, obligation_id, observation_id,
                  scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                  waiver_json, created_at
                ) VALUES (
                  ?1, 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
                  ?2, ?3, ?4, 'temporary outage',
                  '2026-08-01T00:00:00Z', 'reviewer', ?5, '2026-07-28T12:00:00Z'
                )",
                params![
                    waiver_id,
                    scope_kind,
                    scope_id,
                    test_digest(idx as u8 + 16),
                    waiver_json(&waiver_id, scope_kind, scope_id, &["obs-http-200"])
                ],
            )
            .unwrap();
        }

        let persisted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence_waivers
                 WHERE scope_kind IN ('criterion', 'item', 'plan', 'goal')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, 4);
    }

    #[test]
    fn evidence_schema_upgrade_adds_goal_to_existing_waiver_scope_check() {
        let conn = conn();
        conn.execute_batch(
            "CREATE TABLE evidence_waivers(
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              approval_item_id TEXT NOT NULL,
              obligation_id TEXT,
              observation_id TEXT NOT NULL,
              scope_kind TEXT NOT NULL CHECK(scope_kind IN ('project','plan','item','criterion','obligation','observation')),
              scope_id TEXT NOT NULL,
              waiver_digest TEXT NOT NULL UNIQUE,
              reason TEXT NOT NULL,
              expires_at TEXT NOT NULL,
              created_by TEXT NOT NULL,
              waiver_json TEXT NOT NULL,
              created_at TEXT NOT NULL
            );",
        )
        .unwrap();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");

        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'waiver-goal-after-migration', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'goal', 'goal-release', ?1, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
            )",
            params![
                DIGEST,
                waiver_json(
                    "waiver-goal-after-migration",
                    "goal",
                    "goal-release",
                    &["obs-http-200"]
                )
            ],
        )
        .unwrap();
    }

    #[test]
    fn evidence_waivers_require_frozen_contract_json_and_persist_all_observations() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");

        for (id, waiver_json) in [
            ("waiver-empty-json", "{}".to_string()),
            (
                "waiver-mismatched-approval",
                waiver_json(
                    "waiver-mismatched-approval",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace("\"approval_ref\":\"i-approval\"", "\"approval_ref\":\"i-other\""),
            ),
            (
                "waiver-missing-source-target",
                "{\"id\":\"waiver-missing-source-target\",\"schema_version\":\"evidence.contract.v1\",\"scope\":{\"kind\":\"item\",\"id\":\"i-target\"},\"observation_ids\":[\"obs-http-200\"],\"reason\":\"temporary outage\",\"created_by\":\"reviewer\",\"created_at\":\"2026-07-28T12:00:00Z\",\"expires_at\":\"2026-08-01T00:00:00Z\",\"approval_ref\":\"i-approval\",\"audit_trail\":[{\"event\":\"created\"}]}".to_string(),
            ),
            (
                "waiver-empty-source-binding",
                waiver_json(
                    "waiver-empty-source-binding",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!(
                        "\"source\":{{\"revision\":\"0123456789abcdef0123456789abcdef01234567\",\"tree_digest\":\"{DIGEST_C}\",\"dirty\":false}}"
                    ),
                    "\"source\":{}",
                ),
            ),
            (
                "waiver-malformed-target-binding",
                waiver_json(
                    "waiver-malformed-target-binding",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!(
                        "\"target\":{{\"kind\":\"local_service\",\"uri\":\"http://127.0.0.1:4173/health\",\"digest\":\"{DIGEST_B}\"}}"
                    ),
                    "\"target\":{\"kind\":\"local_service\"}",
                ),
            ),
            (
                "waiver-invalid-target-digest",
                waiver_json(
                    "waiver-invalid-target-digest",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!(
                        "\"target\":{{\"kind\":\"local_service\",\"uri\":\"http://127.0.0.1:4173/health\",\"digest\":\"{DIGEST_B}\"}}"
                    ),
                    "\"target\":{\"kind\":\"local_service\",\"uri\":\"http://127.0.0.1:4173/health\",\"digest\":\"sha256:BAD\"}",
                ),
            ),
            (
                "waiver-empty-observations",
                waiver_json("waiver-empty-observations", "item", "i-target", &["obs-http-200"])
                    .replace("\"observation_ids\":[\"obs-http-200\"]", "\"observation_ids\":[]"),
            ),
            (
                "waiver-malformed-audit-entry",
                waiver_json(
                    "waiver-malformed-audit-entry",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!(
                        "\"audit_trail\":[{{\"event\":\"created\",\"at\":\"{WAIVER_CREATED_AT}\"}}]"
                    ),
                    "\"audit_trail\":[\"created\"]",
                ),
            ),
            (
                "waiver-malformed-created-at",
                waiver_json(
                    "waiver-malformed-created-at",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!("\"created_at\":\"{WAIVER_CREATED_AT}\""),
                    "\"created_at\":\"not-a-timestamp\"",
                ),
            ),
            (
                "waiver-created-at-mismatch",
                waiver_json(
                    "waiver-created-at-mismatch",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!("\"created_at\":\"{WAIVER_CREATED_AT}\""),
                    "\"created_at\":\"2026-07-29T12:00:00Z\"",
                ),
            ),
            (
                "waiver-extra-contract-property",
                waiver_json(
                    "waiver-extra-contract-property",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    "\"approval_ref\":\"i-approval\"",
                    "\"approval_ref\":\"i-approval\",\"legacy\":true",
                ),
            ),
        ] {
            assert!(
                conn.execute(
                    "INSERT INTO evidence_waivers(
                      id, project_id, approval_item_id, obligation_id, observation_id,
                      scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
                      waiver_json, created_at
                    ) VALUES (
                      ?1, 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
                      'item', 'i-target', ?2, 'temporary outage',
                      '2026-08-01T00:00:00Z', 'reviewer', ?3, '2026-07-28T12:00:00Z'
                    )",
                    params![id, DIGEST_C, waiver_json],
                )
                .is_err(),
                "{id} should not authorize a waiver"
            );
        }

        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'waiver-multi-observation', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'item', 'i-target', ?1, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
            )",
            params![
                DIGEST,
                waiver_json(
                    "waiver-multi-observation",
                    "item",
                    "i-target",
                    &["obs-http-200", "obs-json-body"]
                )
            ],
        )
        .unwrap();

        let observations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM evidence_waiver_observations
                 WHERE waiver_id = 'waiver-multi-observation'
                   AND observation_id IN ('obs-http-200', 'obs-json-body')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(observations, 2);
    }

    #[test]
    fn evidence_waiver_storage_validator_matches_canonical_schema_edges() {
        let accepted = [
            (
                "waiver-parity-empty-audit-object",
                waiver_json(
                    "waiver-parity-empty-audit-object",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!("{{\"event\":\"created\",\"at\":\"{WAIVER_CREATED_AT}\"}}"),
                    "{}",
                ),
            ),
            (
                "waiver-parity-freeform-audit-at",
                waiver_json(
                    "waiver-parity-freeform-audit-at",
                    "item",
                    "i-target",
                    &["obs-http-200"],
                )
                .replace(
                    &format!("\"at\":\"{WAIVER_CREATED_AT}\""),
                    "\"at\":\"not-a-timestamp\"",
                ),
            ),
        ];
        for (id, waiver_json) in accepted {
            assert_canonical_waiver_schema(&waiver_json, true);
            assert_storage_waiver_contract(id, &waiver_json, true);
        }

        let rejected = [(
            "waiver-parity-short-unicode-revision",
            waiver_json(
                "waiver-parity-short-unicode-revision",
                "item",
                "i-target",
                &["obs-http-200"],
            )
            .replace(
                "\"revision\":\"0123456789abcdef0123456789abcdef01234567\"",
                "\"revision\":\"éééé\"",
            ),
        )];
        for (id, waiver_json) in rejected {
            assert_canonical_waiver_schema(&waiver_json, false);
            assert_storage_waiver_contract(id, &waiver_json, false);
        }
    }

    #[test]
    fn evidence_waivers_are_append_only_after_approval() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");
        let frozen_waiver_json =
            waiver_json("waiver-frozen", "item", "i-target", &["obs-http-200"]);
        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'waiver-frozen', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'item', 'i-target', ?1, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
            )",
            params![DIGEST, frozen_waiver_json],
        )
        .unwrap();

        for sql in [
            "UPDATE evidence_waivers SET expires_at = '2026-09-01T00:00:00Z' WHERE id = 'waiver-frozen'",
            "UPDATE evidence_waivers SET created_by = 'other-reviewer' WHERE id = 'waiver-frozen'",
            "UPDATE evidence_waivers SET reason = 'changed' WHERE id = 'waiver-frozen'",
            "UPDATE evidence_waivers SET waiver_digest = 'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE id = 'waiver-frozen'",
            "UPDATE evidence_waivers SET waiver_json = '{\"binding\":\"changed\"}' WHERE id = 'waiver-frozen'",
            "DELETE FROM evidence_waivers WHERE id = 'waiver-frozen'",
        ] {
            assert!(conn.execute(sql, []).is_err(), "{sql} should be rejected");
        }
        assert!(
            conn.execute(
                "UPDATE evidence_waiver_observations SET observation_id = 'obs-json-body' WHERE waiver_id = 'waiver-frozen'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM evidence_waiver_observations WHERE waiver_id = 'waiver-frozen'",
                [],
            )
            .is_err()
        );
        for sql in [
            "INSERT INTO evidence_waiver_observations(waiver_id, observation_id, ordinal, created_at)
             VALUES ('waiver-frozen', 'obs-json-body', 1, '2026-07-28T12:00:00Z')",
            "INSERT INTO evidence_waiver_observations(waiver_id, observation_id, ordinal, created_at)
             VALUES ('waiver-frozen', 'obs-arbitrary', 1, '2026-07-28T12:00:00Z')",
            "INSERT INTO evidence_waiver_observations(waiver_id, observation_id, ordinal, created_at)
             VALUES ('waiver-frozen', 'obs-http-200', 1, '2026-07-28T12:00:00Z')",
        ] {
            assert!(conn.execute(sql, []).is_err(), "{sql} should be rejected");
        }
    }

    #[test]
    fn legacy_waiver_audit_requires_child_rows_to_exactly_match_parent_json() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);
        approve_item(&conn, "i-approval");
        conn.execute(
            "INSERT INTO evidence_waivers(
              id, project_id, approval_item_id, obligation_id, observation_id,
              scope_kind, scope_id, waiver_digest, reason, expires_at, created_by,
              waiver_json, created_at
            ) VALUES (
              'waiver-legacy-extra-child', 'p-evidence', 'i-approval', 'obl-1', 'obs-http-200',
              'item', 'i-target', ?1, 'temporary outage',
              '2026-08-01T00:00:00Z', 'reviewer', ?2, '2026-07-28T12:00:00Z'
            )",
            params![
                DIGEST,
                waiver_json(
                    "waiver-legacy-extra-child",
                    "item",
                    "i-target",
                    &["obs-http-200"]
                )
            ],
        )
        .unwrap();
        conn.execute_batch("DROP TRIGGER evidence_waiver_observations_validate_parent_json")
            .unwrap();
        conn.execute(
            "INSERT INTO evidence_waiver_observations(
              waiver_id, observation_id, ordinal, created_at
            ) VALUES (
              'waiver-legacy-extra-child', 'obs-json-body', 1, '2026-07-28T12:00:00Z'
            )",
            [],
        )
        .unwrap();

        let error = super::backfill_evidence_waiver_observations(&conn)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("non-authoritative observations for waiver-legacy-extra-child"),
            "{error}"
        );
    }

    #[test]
    fn evidence_attempts_receipts_and_instances_require_frozen_bindings() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);

        assert!(
            conn.execute(
                "INSERT INTO verification_capability_instances(
                  id, manifest_id, manifest_version, manifest_digest, probe_execution_id,
                  availability_status, runtime_target_json, capability_snapshot_json,
                  probe_result_json, created_at
                ) VALUES (
                  'cap-instance-bad', 'cap.manifest', '1.0.0', ?1, 'probe-bad',
                  'available', '{}', '{}', '{}', datetime('now')
                )",
                params![DIGEST_C],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_attempts(
                  id, project_id, obligation_id, capability_instance_id, attempt_status,
                  execution_contract_digest, resolved_command_json, environment_digest,
                  started_at, attempt_json, created_at
                ) VALUES (
                  'attempt-unbound-obligation', 'p-evidence', NULL, 'cap-instance-1', 'passed',
                  ?1, '{}', ?2, datetime('now'), '{}', datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO evidence_attempts(
                  id, project_id, obligation_id, capability_instance_id, attempt_status,
                  execution_contract_digest, resolved_command_json, environment_digest,
                  started_at, attempt_json, created_at
                ) VALUES (
                  'attempt-unbound-capability', 'p-evidence', 'obl-1', NULL, 'passed',
                  ?1, '{}', ?2, datetime('now'), '{}', datetime('now')
                )",
                params![DIGEST, DIGEST_B],
            )
            .is_err()
        );
        insert_attempt(&conn);
        assert!(
            conn.execute(
                "INSERT INTO evidence_receipts(
                  id, project_id, obligation_id, attempt_id, receipt_status, receipt_digest,
                  trusted_binding_json, observations_json, provenance_json, receipt_json, created_at
                ) VALUES (
                  'receipt-unbound', 'p-evidence', NULL, 'attempt-1', 'trusted', ?1,
                  '{}', '[]', '{}', '{}', datetime('now')
                )",
                params![DIGEST],
            )
            .is_err()
        );
    }

    #[test]
    fn verification_capability_manifests_are_immutable_snapshots() {
        let conn = conn();
        ensure_schema(&conn).unwrap();
        seed_project_and_item(&conn);
        seed_binding(&conn);

        assert!(
            conn.execute(
                "UPDATE verification_capability_manifests SET manifest_digest = ?1 WHERE id = 'cap.manifest'",
                params![DIGEST_C],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "DELETE FROM verification_capability_manifests WHERE id = 'cap.manifest'",
                [],
            )
            .is_err()
        );
    }
}
