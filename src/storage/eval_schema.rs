use anyhow::Result;
use rusqlite::Connection;

pub(super) fn ensure_eval_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS eval_suite_snapshots(
  digest TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  suite_id TEXT NOT NULL,
  suite_version TEXT NOT NULL,
  normalized_manifest TEXT NOT NULL,
  fixture_digests TEXT NOT NULL,
  scorer_identities TEXT NOT NULL,
  case_order TEXT NOT NULL,
  baseline_selector TEXT NOT NULL,
  comparison_policy_digest TEXT NOT NULL,
  metadata TEXT NOT NULL DEFAULT '{}',
  source_path TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(project_id, suite_id, suite_version, digest)
);
CREATE TABLE IF NOT EXISTS eval_runs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  suite_digest TEXT NOT NULL,
  subject_kind TEXT NOT NULL,
  subject_revision TEXT NOT NULL,
  subject_path TEXT,
  subject_argv TEXT NOT NULL,
  subject_label TEXT,
  runner_version TEXT NOT NULL,
  planr_version TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('started','success','error','inconclusive','invalidated')),
  created_at TEXT NOT NULL,
  started_at TEXT,
  completed_at TEXT,
  testbed_fingerprint TEXT NOT NULL,
  source_state TEXT NOT NULL,
  case_counts TEXT NOT NULL,
  parent_run_id TEXT,
  resume_of TEXT,
  rescore_of TEXT,
  recompute_of TEXT,
  invalidated_by TEXT,
  aggregate_summary TEXT NOT NULL DEFAULT '{}',
  FOREIGN KEY(suite_digest) REFERENCES eval_suite_snapshots(digest)
);
CREATE TABLE IF NOT EXISTS eval_case_results(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  case_id TEXT NOT NULL,
  scorer_id TEXT NOT NULL,
  scorer_version TEXT NOT NULL,
  fixture_digest TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pass','fail','error','skipped','inconclusive')),
  repetition_count INTEGER NOT NULL CHECK(repetition_count >= 0),
  warmup_count INTEGER NOT NULL CHECK(warmup_count >= 0),
  assertions TEXT NOT NULL DEFAULT '[]',
  command TEXT NOT NULL,
  sample_ids TEXT NOT NULL DEFAULT '[]',
  reasons TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES eval_runs(id),
  UNIQUE(run_id, case_id, scorer_id, scorer_version)
);
CREATE TABLE IF NOT EXISTS eval_attempts(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  case_id TEXT NOT NULL,
  scorer_id TEXT NOT NULL,
  scorer_version TEXT NOT NULL,
  repetition_index INTEGER NOT NULL CHECK(repetition_index >= 0),
  warmup INTEGER NOT NULL CHECK(warmup IN (0, 1)),
  attempt_index INTEGER NOT NULL CHECK(attempt_index >= 0),
  seed INTEGER NOT NULL,
  terminal_status TEXT NOT NULL CHECK(terminal_status IN ('verified_success','pass','failed','fail','error','skipped','inconclusive','stopped')),
  countable INTEGER NOT NULL DEFAULT 1 CHECK(countable IN (0, 1)),
  retry_of_attempt_id TEXT,
  fallback_of_attempt_id TEXT,
  escalation_of_attempt_id TEXT,
  resume_of_attempt_id TEXT,
  effective_client TEXT,
  effective_provider TEXT,
  effective_runtime TEXT,
  effective_model TEXT,
  effective_effort TEXT,
  effective_profile_id TEXT,
  profile_config_digest TEXT,
  runner_harness_version TEXT NOT NULL,
  route_observation_json TEXT,
  outcome_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES eval_runs(id),
  UNIQUE(run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index)
);
CREATE TABLE IF NOT EXISTS eval_samples(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  case_id TEXT NOT NULL,
  scorer_id TEXT NOT NULL,
  scorer_version TEXT NOT NULL,
  repetition_index INTEGER NOT NULL CHECK(repetition_index >= 0),
  warmup INTEGER NOT NULL CHECK(warmup IN (0, 1)),
  seed INTEGER NOT NULL,
  measure TEXT NOT NULL,
  value_json TEXT NOT NULL,
  unit TEXT NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('process','planr_event','scorer','metering')),
  attempt_id TEXT NOT NULL,
  attempt_index INTEGER NOT NULL DEFAULT 0 CHECK(attempt_index >= 0),
  metering_basis TEXT NOT NULL DEFAULT 'actual_trusted' CHECK(metering_basis IN ('actual_trusted','estimated','mixed','unavailable')),
  basis_source TEXT NOT NULL DEFAULT 'process',
  basis_confidence TEXT NOT NULL DEFAULT 'verified' CHECK(basis_confidence IN ('verified','estimated','mixed','unavailable')),
  estimate_provenance TEXT,
  captured_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES eval_runs(id),
  FOREIGN KEY(attempt_id) REFERENCES eval_attempts(id),
  UNIQUE(run_id, case_id, scorer_id, scorer_version, attempt_id, measure)
);
CREATE TABLE IF NOT EXISTS eval_comparisons(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  baseline_run_id TEXT NOT NULL,
  candidate_run_id TEXT NOT NULL,
  policy_digest TEXT NOT NULL,
  runner_version TEXT NOT NULL,
  verdict TEXT NOT NULL CHECK(verdict IN ('improved','regressed','no_material_difference','insufficient_evidence')),
  reasons TEXT NOT NULL DEFAULT '[]',
  gates TEXT NOT NULL DEFAULT '[]',
  effect_estimates TEXT NOT NULL DEFAULT '{}',
  uncertainty TEXT NOT NULL DEFAULT '{}',
  protected_dimensions TEXT NOT NULL DEFAULT '{}',
  recompute_of TEXT,
  rescore_of TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(baseline_run_id) REFERENCES eval_runs(id),
  FOREIGN KEY(candidate_run_id) REFERENCES eval_runs(id)
);
CREATE TABLE IF NOT EXISTS eval_invalidations(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  target_kind TEXT NOT NULL CHECK(target_kind IN ('run','comparison')),
  target_id TEXT NOT NULL,
  reason TEXT NOT NULL,
  reason_codes TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL,
  created_by TEXT NOT NULL,
  replacement_hint TEXT
);
CREATE TABLE IF NOT EXISTS eval_evidence_refs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  target_kind TEXT NOT NULL CHECK(target_kind IN ('run','comparison')),
  target_id TEXT NOT NULL,
  planr_attachment_kind TEXT NOT NULL CHECK(planr_attachment_kind IN ('log','review','artifact')),
  planr_attachment_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  closure_authority INTEGER NOT NULL DEFAULT 0 CHECK(closure_authority = 0),
  created_at TEXT NOT NULL
);
"#,
    )?;
    ensure_eval_sample_column(
        conn,
        "attempt_id",
        "ALTER TABLE eval_samples ADD COLUMN attempt_id TEXT",
    )?;
    ensure_eval_sample_column(
        conn,
        "attempt_index",
        "ALTER TABLE eval_samples ADD COLUMN attempt_index INTEGER NOT NULL DEFAULT 0 CHECK(attempt_index >= 0)",
    )?;
    ensure_eval_sample_column(
        conn,
        "metering_basis",
        "ALTER TABLE eval_samples ADD COLUMN metering_basis TEXT NOT NULL DEFAULT 'actual_trusted' CHECK(metering_basis IN ('actual_trusted','estimated','mixed','unavailable'))",
    )?;
    ensure_eval_sample_column(
        conn,
        "basis_source",
        "ALTER TABLE eval_samples ADD COLUMN basis_source TEXT NOT NULL DEFAULT 'process'",
    )?;
    ensure_eval_sample_column(
        conn,
        "basis_confidence",
        "ALTER TABLE eval_samples ADD COLUMN basis_confidence TEXT NOT NULL DEFAULT 'verified' CHECK(basis_confidence IN ('verified','estimated','mixed','unavailable'))",
    )?;
    migrate_eval_samples_basis_confidence(conn)?;
    migrate_eval_attempt_terminal_status_vocab(conn)?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS eval_samples_attempt_measure_idx ON eval_samples(run_id, case_id, scorer_id, scorer_version, attempt_id, measure) WHERE attempt_id IS NOT NULL",
        [],
    )?;
    ensure_eval_sample_column(
        conn,
        "estimate_provenance",
        "ALTER TABLE eval_samples ADD COLUMN estimate_provenance TEXT",
    )?;
    Ok(())
}

fn migrate_eval_attempt_terminal_status_vocab(conn: &Connection) -> Result<()> {
    let create_sql: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'eval_attempts'",
        [],
        |row| row.get(0),
    )?;
    let Some(create_sql) = create_sql else {
        return Ok(());
    };
    if create_sql.contains("verified_success") {
        return Ok(());
    }
    let foreign_keys_enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    conn.execute_batch(
        r#"
DROP TABLE IF EXISTS eval_attempts_migrated;
CREATE TABLE eval_attempts_migrated(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  case_id TEXT NOT NULL,
  scorer_id TEXT NOT NULL,
  scorer_version TEXT NOT NULL,
  repetition_index INTEGER NOT NULL CHECK(repetition_index >= 0),
  warmup INTEGER NOT NULL CHECK(warmup IN (0, 1)),
  attempt_index INTEGER NOT NULL CHECK(attempt_index >= 0),
  seed INTEGER NOT NULL,
  terminal_status TEXT NOT NULL CHECK(terminal_status IN ('verified_success','pass','failed','fail','error','skipped','inconclusive','stopped')),
  countable INTEGER NOT NULL DEFAULT 1 CHECK(countable IN (0, 1)),
  retry_of_attempt_id TEXT,
  fallback_of_attempt_id TEXT,
  escalation_of_attempt_id TEXT,
  resume_of_attempt_id TEXT,
  effective_client TEXT,
  effective_provider TEXT,
  effective_runtime TEXT,
  effective_model TEXT,
  effective_effort TEXT,
  effective_profile_id TEXT,
  profile_config_digest TEXT,
  runner_harness_version TEXT NOT NULL,
  route_observation_json TEXT,
  outcome_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES eval_runs(id),
  UNIQUE(run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, attempt_index)
);
INSERT INTO eval_attempts_migrated(
  id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup,
  attempt_index, seed, terminal_status, countable, retry_of_attempt_id,
  fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id,
  effective_client, effective_provider, effective_runtime, effective_model,
  effective_effort, effective_profile_id, profile_config_digest,
  runner_harness_version, route_observation_json, outcome_json, created_at
)
SELECT
  id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup,
  attempt_index, seed, terminal_status, countable, retry_of_attempt_id,
  fallback_of_attempt_id, escalation_of_attempt_id, resume_of_attempt_id,
  effective_client, effective_provider, effective_runtime, effective_model,
  effective_effort, effective_profile_id, profile_config_digest,
  runner_harness_version, route_observation_json, outcome_json, created_at
FROM eval_attempts;
DROP TABLE eval_attempts;
ALTER TABLE eval_attempts_migrated RENAME TO eval_attempts;
"#,
    )?;
    if foreign_keys_enabled != 0 {
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
    }
    Ok(())
}

fn ensure_eval_sample_column(conn: &Connection, name: &str, sql: &str) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(eval_samples)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == name {
            return Ok(());
        }
    }
    conn.execute_batch(sql)?;
    Ok(())
}

fn migrate_eval_samples_basis_confidence(conn: &Connection) -> Result<()> {
    let create_sql: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'eval_samples'",
        [],
        |row| row.get(0),
    )?;
    let Some(create_sql) = create_sql else {
        return Ok(());
    };
    if !create_sql.contains("basis_confidence")
        || !create_sql.contains("actual_trusted")
        || create_sql.contains("'verified','estimated','mixed','unavailable'")
    {
        return Ok(());
    }
    let foreign_keys_enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    conn.execute_batch(
        r#"
DROP TABLE IF EXISTS eval_samples_migrated;
CREATE TABLE eval_samples_migrated(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  case_id TEXT NOT NULL,
  scorer_id TEXT NOT NULL,
  scorer_version TEXT NOT NULL,
  repetition_index INTEGER NOT NULL CHECK(repetition_index >= 0),
  warmup INTEGER NOT NULL CHECK(warmup IN (0, 1)),
  seed INTEGER NOT NULL,
  measure TEXT NOT NULL,
  value_json TEXT NOT NULL,
  unit TEXT NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('process','planr_event','scorer','metering')),
  attempt_id TEXT NOT NULL,
  attempt_index INTEGER NOT NULL DEFAULT 0 CHECK(attempt_index >= 0),
  metering_basis TEXT NOT NULL DEFAULT 'actual_trusted' CHECK(metering_basis IN ('actual_trusted','estimated','mixed','unavailable')),
  basis_source TEXT NOT NULL DEFAULT 'process',
  basis_confidence TEXT NOT NULL DEFAULT 'verified' CHECK(basis_confidence IN ('verified','estimated','mixed','unavailable')),
  estimate_provenance TEXT,
  captured_at TEXT NOT NULL,
  FOREIGN KEY(run_id) REFERENCES eval_runs(id),
  FOREIGN KEY(attempt_id) REFERENCES eval_attempts(id),
  UNIQUE(run_id, case_id, scorer_id, scorer_version, attempt_id, measure)
);
INSERT INTO eval_samples_migrated(
  id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed,
  measure, value_json, unit, source, attempt_id, attempt_index, metering_basis,
  basis_source, basis_confidence, estimate_provenance, captured_at
)
SELECT
  id, run_id, case_id, scorer_id, scorer_version, repetition_index, warmup, seed,
  measure, value_json, unit, source, attempt_id, attempt_index, metering_basis,
  basis_source,
  CASE basis_confidence WHEN 'actual_trusted' THEN 'verified' ELSE basis_confidence END,
  estimate_provenance, captured_at
FROM eval_samples;
DROP TABLE eval_samples;
ALTER TABLE eval_samples_migrated RENAME TO eval_samples;
"#,
    )?;
    if foreign_keys_enabled != 0 {
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
    }
    Ok(())
}
