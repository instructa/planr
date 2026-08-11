use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug)]
struct LegacyReviewStep {
    review_id: String,
    target_id: String,
}

const LEGACY_REVIEW_SETTLEMENT_MARKER: &str = "execution_run_legacy_review_settlement_version";
const LEGACY_REVIEW_SETTLEMENT_VERSION: &str = "1";

/// Installs the first-class FeatureRun storage boundary atomically.
///
/// Existing graph-era rows are intentionally neither copied nor interpreted:
/// they remain historical records in their original tables while every new
/// execution producer writes only these canonical tables.
pub(super) fn ensure_execution_run_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("SAVEPOINT execution_run_schema_upgrade")?;
    let result = conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS feature_runs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('active','held','complete','cancelled')),
  phase TEXT NOT NULL CHECK(phase IN ('implementation','risk_review','source_frozen','verification','final_review','complete','held','cancelled')),
  policy_digest TEXT NOT NULL,
  budget_contract_digest TEXT,
  source_revision TEXT,
  active_batch_id TEXT,
  outcomes_settled INTEGER NOT NULL DEFAULT 0 CHECK(outcomes_settled >= 0),
  batch_outcome_count INTEGER NOT NULL DEFAULT 0 CHECK(batch_outcome_count >= 0),
  held_from_phase TEXT CHECK(held_from_phase IN ('implementation','risk_review','source_frozen','verification','final_review')),
  hold_reason TEXT CHECK(hold_reason IN ('budget','capability')),
  terminal_reason TEXT CHECK(terminal_reason IN ('completed','user_cancelled','policy_cancelled')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(active_batch_id, id) REFERENCES execution_batches(id, run_id) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS feature_run_budget_contracts(
  run_id TEXT PRIMARY KEY REFERENCES feature_runs(id) ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED,
  schema TEXT NOT NULL CHECK(schema = 'planr.feature_run_budget_contract.v2'),
  digest TEXT NOT NULL UNIQUE CHECK(length(digest) = 71 AND substr(digest, 1, 7) = 'sha256:' AND substr(digest, 8) NOT GLOB '*[^0-9a-f]*'),
  contract_json TEXT NOT NULL CHECK(json_valid(contract_json)),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(run_id, digest),
  CHECK(json_extract(contract_json, '$.run_id') = run_id),
  CHECK(json_extract(contract_json, '$.schema') = schema),
  CHECK(json_extract(contract_json, '$.digest') = digest),
  CHECK(json_extract(contract_json, '$.started_at_unix_ms') > 0),
  CHECK(json_extract(contract_json, '$.mode') IN ('bounded','unbounded'))
);

CREATE TABLE IF NOT EXISTS execution_batches(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
  maker_worker_id TEXT NOT NULL CHECK(length(trim(maker_worker_id)) > 0),
  status TEXT NOT NULL CHECK(status IN ('active','paused_for_risk_review','ended')),
  replaced_maker_worker_id TEXT,
  successor_maker_worker_id TEXT,
  replacement_reason TEXT CHECK(replacement_reason IN ('unavailable','context_lost','ownership_incompatible','batch_cap_reached')),
  replacement_reference TEXT,
  replacement_explanation TEXT,
  revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
  started_at TEXT NOT NULL DEFAULT (datetime('now')),
  ended_at TEXT,
  UNIQUE(id, run_id),
  CHECK(
    (replacement_reason IS NULL AND replaced_maker_worker_id IS NULL AND successor_maker_worker_id IS NULL AND replacement_reference IS NULL AND replacement_explanation IS NULL)
    OR
    (status = 'ended' AND replacement_reason IS NOT NULL AND replaced_maker_worker_id IS NOT NULL AND successor_maker_worker_id IS NOT NULL AND replacement_reference IS NOT NULL AND replacement_explanation IS NOT NULL AND replaced_maker_worker_id = maker_worker_id AND length(trim(replaced_maker_worker_id)) > 0 AND length(trim(successor_maker_worker_id)) > 0 AND length(trim(replacement_reference)) > 0 AND length(trim(replacement_explanation)) > 0 AND replaced_maker_worker_id <> successor_maker_worker_id)
  )
);
CREATE INDEX IF NOT EXISTS idx_execution_batches_run ON execution_batches(run_id, started_at, id);

CREATE TABLE IF NOT EXISTS execution_run_outcomes(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  batch_id TEXT NOT NULL REFERENCES execution_batches(id) ON DELETE CASCADE,
  item_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal >= 1),
  outcome_json TEXT NOT NULL CHECK(json_valid(outcome_json)),
  settled_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(run_id, item_id),
  UNIQUE(batch_id, ordinal)
);

CREATE TABLE IF NOT EXISTS feature_run_role_leases(
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('maker','verifier','reviewer')),
  worker_id TEXT NOT NULL,
  lease_generation INTEGER NOT NULL CHECK(lease_generation >= 1),
  acquired_at TEXT NOT NULL DEFAULT (datetime('now')),
  released_at TEXT,
  PRIMARY KEY(run_id, role, lease_generation)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_feature_run_role_leases_active
  ON feature_run_role_leases(run_id, role) WHERE released_at IS NULL;

CREATE TABLE IF NOT EXISTS review_gates(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  scope_kind TEXT NOT NULL CHECK(scope_kind IN ('outcome','plan')),
  scope_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('risk_checkpoint','final_product')),
  status TEXT NOT NULL CHECK(status IN ('pending','leased','accepted','changes_requested','cancelled')),
  required_risk TEXT,
  responsible_maker_id TEXT NOT NULL,
  latest_attempt INTEGER NOT NULL DEFAULT 0 CHECK(latest_attempt >= 0),
  source_revision TEXT,
  accepted_at TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  CHECK(kind <> 'final_product' OR (source_revision IS NOT NULL AND length(trim(source_revision)) > 0)),
  UNIQUE(run_id, kind, scope_kind, scope_id),
  UNIQUE(id, run_id)
);

CREATE TABLE IF NOT EXISTS final_review_source_bindings(
  gate_id TEXT PRIMARY KEY REFERENCES review_gates(id) ON DELETE CASCADE,
  freeze_id TEXT NOT NULL REFERENCES feature_run_source_freezes(id) ON DELETE RESTRICT,
  source_revision TEXT NOT NULL CHECK(length(trim(source_revision)) > 0),
  source_digest TEXT NOT NULL CHECK(length(trim(source_digest)) > 0),
  receipt_lineage_json TEXT NOT NULL CHECK(json_valid(receipt_lineage_json)),
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS review_attempts(
  id TEXT PRIMARY KEY,
  gate_id TEXT NOT NULL REFERENCES review_gates(id) ON DELETE CASCADE,
  attempt_number INTEGER NOT NULL CHECK(attempt_number >= 1),
  reviewer_worker_id TEXT NOT NULL,
  reviewer_mode TEXT NOT NULL,
  verdict TEXT NOT NULL CHECK(verdict IN ('accepted','changes_requested','blocked')),
  source_revision TEXT NOT NULL CHECK(length(trim(source_revision)) > 0),
  artifacts_json TEXT NOT NULL CHECK(json_valid(artifacts_json)),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(gate_id, attempt_number)
);

CREATE TABLE IF NOT EXISTS review_findings(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  gate_id TEXT NOT NULL,
  attempt_id TEXT NOT NULL REFERENCES review_attempts(id) ON DELETE CASCADE,
  severity TEXT NOT NULL CHECK(severity IN ('low','moderate','high','critical')),
  target TEXT NOT NULL,
  owner_worker_id TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('open','resolved','dismissed')),
  invalidated_evidence_ids_json TEXT NOT NULL CHECK(json_valid(invalidated_evidence_ids_json)),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  resolved_at TEXT,
  UNIQUE(id, run_id),
  FOREIGN KEY(gate_id, run_id) REFERENCES review_gates(id, run_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_review_findings_gate_status ON review_findings(gate_id, status, id);

CREATE TABLE IF NOT EXISTS feature_run_budget_observations(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  reservation_id TEXT,
  sequence INTEGER CHECK(sequence IS NULL OR sequence >= 1),
  phase TEXT NOT NULL CHECK(phase IN ('implementation','verification','review','repair')),
  metering TEXT NOT NULL CHECK(metering IN ('unavailable','estimated','trusted')),
  wall_metering TEXT CHECK(wall_metering IS NULL OR wall_metering IN ('unavailable','estimated','trusted')),
  tool_calls_metering TEXT CHECK(tool_calls_metering IS NULL OR tool_calls_metering IN ('unavailable','estimated','trusted')),
  tokens_metering TEXT CHECK(tokens_metering IS NULL OR tokens_metering IN ('unavailable','estimated','trusted')),
  wall_seconds INTEGER CHECK(wall_seconds IS NULL OR wall_seconds >= 0),
  tokens INTEGER CHECK(tokens IS NULL OR tokens >= 0),
  tool_calls INTEGER CHECK(tool_calls IS NULL OR tool_calls >= 0),
  credits_micros INTEGER CHECK(credits_micros IS NULL OR credits_micros >= 0),
  provenance TEXT NOT NULL CHECK(length(trim(provenance)) > 0),
  adapter_id TEXT,
  observed_at_unix_ms INTEGER CHECK(observed_at_unix_ms IS NULL OR observed_at_unix_ms > 0),
  observed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_feature_run_budget_observations_run ON feature_run_budget_observations(run_id, observed_at, id);

CREATE TABLE IF NOT EXISTS feature_run_budget_reservations(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  contract_digest TEXT,
  phase TEXT NOT NULL CHECK(phase IN ('implementation','verification','review','repair')),
  boundary_key TEXT NOT NULL CHECK(length(trim(boundary_key)) > 0),
  owner_role TEXT CHECK(owner_role IS NULL OR owner_role IN ('maker','verifier','reviewer')),
  owner_worker_id TEXT,
  lease_generation INTEGER CHECK(lease_generation IS NULL OR lease_generation >= 1),
  status TEXT NOT NULL CHECK(status IN ('active','reconciled','released')),
  reserved_wall_seconds INTEGER CHECK(reserved_wall_seconds IS NULL OR reserved_wall_seconds >= 1),
  reserved_tokens INTEGER CHECK(reserved_tokens IS NULL OR reserved_tokens >= 1),
  reserved_tool_calls INTEGER CHECK(reserved_tool_calls IS NULL OR reserved_tool_calls >= 1),
  deadline_unix_ms INTEGER CHECK(deadline_unix_ms IS NULL OR deadline_unix_ms > 0),
  started_at_unix_ms INTEGER NOT NULL CHECK(started_at_unix_ms >= 0),
  finished_at TEXT,
  provenance TEXT NOT NULL CHECK(length(trim(provenance)) > 0)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_feature_run_budget_reservations_active
  ON feature_run_budget_reservations(run_id, boundary_key) WHERE status = 'active';

CREATE TABLE IF NOT EXISTS feature_run_source_freezes(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  source_revision TEXT NOT NULL,
  source_digest TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('active','invalidated')),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  invalidated_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_feature_run_source_freezes_active
  ON feature_run_source_freezes(run_id) WHERE status = 'active';

CREATE TABLE IF NOT EXISTS feature_run_evidence_invalidations(
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  freeze_id TEXT NOT NULL REFERENCES feature_run_source_freezes(id) ON DELETE CASCADE,
  finding_id TEXT,
  reason TEXT NOT NULL,
  affected_evidence_ids_json TEXT NOT NULL CHECK(json_valid(affected_evidence_ids_json)),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(finding_id, run_id) REFERENCES review_findings(id, run_id)
);
CREATE INDEX IF NOT EXISTS idx_feature_run_invalidations_run ON feature_run_evidence_invalidations(run_id, created_at, id);

CREATE TABLE IF NOT EXISTS feature_run_product_repair_settlements(
  invalidation_id TEXT PRIMARY KEY REFERENCES feature_run_evidence_invalidations(id),
  run_id TEXT NOT NULL REFERENCES feature_runs(id) ON DELETE CASCADE,
  responsible_maker_id TEXT NOT NULL,
  verification_item_id TEXT NOT NULL REFERENCES items(id),
  selective_obligation_ids_json TEXT NOT NULL CHECK(json_valid(selective_obligation_ids_json)),
  settlement_json TEXT NOT NULL CHECK(json_valid(settlement_json)),
  source_freeze_id TEXT NOT NULL REFERENCES feature_run_source_freezes(id),
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(invalidation_id, run_id)
);

-- One-shot hard-cut settlement starts from stranded targets and follows
-- historical graph edges forward. These persisted indexes keep that bounded
-- probe cheap on established databases without changing legacy row meaning.
CREATE INDEX IF NOT EXISTS idx_links_to_kind_from ON links(to_item, kind, from_item);
CREATE INDEX IF NOT EXISTS idx_logs_item_kind_created ON logs(item_id, kind, created_at, id);
CREATE INDEX IF NOT EXISTS idx_events_item_type_timestamp ON events(item_id, event_type, timestamp, id);
"#,
    );
    let result = result
        .and_then(|()| ensure_execution_run_additive_columns(conn))
        .and_then(|()| ensure_budget_storage_integrity(conn))
        .and_then(|()| migrate_accepted_legacy_review_chains_once(conn));
    match result {
        Ok(()) => {
            conn.execute_batch("RELEASE execution_run_schema_upgrade")?;
            Ok(())
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO execution_run_schema_upgrade; RELEASE execution_run_schema_upgrade",
            );
            Err(error.into())
        }
    }
}

fn ensure_execution_run_additive_columns(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "feature_runs", "budget_contract_digest", "TEXT")?;
    if !column_exists(conn, "feature_runs", "hold_reason")? {
        conn.execute_batch(
            "ALTER TABLE feature_runs ADD COLUMN hold_reason TEXT CHECK(hold_reason IN ('budget','capability'));",
        )?;
    }
    for (column, definition) in [
        ("contract_digest", "TEXT"),
        ("owner_role", "TEXT"),
        ("owner_worker_id", "TEXT"),
        ("lease_generation", "INTEGER"),
        ("deadline_unix_ms", "INTEGER"),
    ] {
        add_column_if_missing(conn, "feature_run_budget_reservations", column, definition)?;
    }
    for (column, definition) in [
        ("reservation_id", "TEXT"),
        ("sequence", "INTEGER"),
        ("wall_metering", "TEXT"),
        ("tool_calls_metering", "TEXT"),
        ("tokens_metering", "TEXT"),
        ("adapter_id", "TEXT"),
        ("observed_at_unix_ms", "INTEGER"),
    ] {
        add_column_if_missing(conn, "feature_run_budget_observations", column, definition)?;
    }
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    if !column_exists(conn, table, column)? {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition};"
        ))?;
    }
    Ok(())
}

fn ensure_budget_storage_integrity(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_feature_run_budget_observation_sequence
  ON feature_run_budget_observations(reservation_id, sequence)
  WHERE reservation_id IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS feature_runs_budget_contract_digest_no_update
BEFORE UPDATE OF budget_contract_digest ON feature_runs
WHEN OLD.budget_contract_digest IS NOT NEW.budget_contract_digest
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget contract binding is immutable');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_contracts_match_run
BEFORE INSERT ON feature_run_budget_contracts
WHEN NOT EXISTS (
  SELECT 1 FROM feature_runs
  WHERE id = NEW.run_id AND budget_contract_digest = NEW.digest
)
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget contract must match its run binding');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_contracts_no_update
BEFORE UPDATE ON feature_run_budget_contracts
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget contracts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_contracts_no_delete
BEFORE DELETE ON feature_run_budget_contracts
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget contracts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_reservations_v2_integrity
BEFORE INSERT ON feature_run_budget_reservations
WHEN NEW.contract_digest IS NOT NULL AND (
  NEW.owner_role IS NULL OR length(trim(NEW.owner_worker_id)) = 0 OR NEW.lease_generation IS NULL
  OR NOT EXISTS (
    SELECT 1 FROM feature_run_budget_contracts AS contract
    WHERE contract.run_id = NEW.run_id AND contract.digest = NEW.contract_digest
  )
  OR NOT EXISTS (
    SELECT 1 FROM feature_run_role_leases AS lease
    WHERE lease.run_id = NEW.run_id
      AND lease.role = NEW.owner_role
      AND lease.worker_id = NEW.owner_worker_id
      AND lease.lease_generation = NEW.lease_generation
      AND lease.released_at IS NULL
  )
  OR EXISTS (
    SELECT 1 FROM feature_run_budget_contracts AS contract
    WHERE contract.run_id = NEW.run_id
      AND json_extract(contract.contract_json, '$.mode') = 'bounded'
      AND (
        NEW.reserved_wall_seconds IS NULL OR NEW.reserved_tool_calls IS NULL
        OR NEW.reserved_tokens IS NULL OR NEW.deadline_unix_ms IS NULL
      )
  )
  OR EXISTS (
    SELECT 1 FROM feature_run_budget_contracts AS contract
    WHERE contract.run_id = NEW.run_id
      AND json_extract(contract.contract_json, '$.mode') = 'unbounded'
      AND (
        NEW.reserved_wall_seconds IS NOT NULL OR NEW.reserved_tool_calls IS NOT NULL
        OR NEW.reserved_tokens IS NOT NULL OR NEW.deadline_unix_ms IS NOT NULL
      )
  )
)
BEGIN
  SELECT RAISE(ABORT, 'invalid FeatureRun budget reservation ownership or contract binding');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_reservations_v2_identity_no_update
BEFORE UPDATE ON feature_run_budget_reservations
WHEN OLD.contract_digest IS NOT NULL AND (
  OLD.id IS NOT NEW.id OR OLD.run_id IS NOT NEW.run_id
  OR OLD.contract_digest IS NOT NEW.contract_digest OR OLD.phase IS NOT NEW.phase
  OR OLD.boundary_key IS NOT NEW.boundary_key OR OLD.owner_role IS NOT NEW.owner_role
  OR OLD.owner_worker_id IS NOT NEW.owner_worker_id
  OR OLD.lease_generation IS NOT NEW.lease_generation
  OR OLD.reserved_wall_seconds IS NOT NEW.reserved_wall_seconds
  OR OLD.reserved_tool_calls IS NOT NEW.reserved_tool_calls
  OR OLD.reserved_tokens IS NOT NEW.reserved_tokens
  OR OLD.deadline_unix_ms IS NOT NEW.deadline_unix_ms
  OR OLD.started_at_unix_ms IS NOT NEW.started_at_unix_ms
  OR OLD.provenance IS NOT NEW.provenance
)
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget reservation identity is immutable');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_reservations_state_monotonic
BEFORE UPDATE OF status ON feature_run_budget_reservations
WHEN OLD.contract_digest IS NOT NULL AND (
  OLD.status <> 'active' OR NEW.status NOT IN ('reconciled','released')
)
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget reservation state is terminal and monotonic');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_observations_v2_integrity
BEFORE INSERT ON feature_run_budget_observations
WHEN NEW.reservation_id IS NOT NULL AND (
  NEW.sequence IS NULL OR NEW.wall_metering IS NULL
  OR NEW.tool_calls_metering IS NULL OR NEW.tokens_metering IS NULL
  OR length(trim(NEW.adapter_id)) = 0 OR NEW.observed_at_unix_ms IS NULL
  OR NEW.metering <> CASE
       WHEN NEW.wall_metering = 'unavailable'
         OR NEW.tool_calls_metering = 'unavailable'
         OR NEW.tokens_metering = 'unavailable' THEN 'unavailable'
       WHEN NEW.wall_metering = 'estimated'
         OR NEW.tool_calls_metering = 'estimated'
         OR NEW.tokens_metering = 'estimated' THEN 'estimated'
       ELSE 'trusted'
     END
  OR (NEW.wall_metering = 'unavailable' AND NEW.wall_seconds IS NOT NULL)
  OR (NEW.wall_metering <> 'unavailable' AND NEW.wall_seconds IS NULL)
  OR (NEW.tool_calls_metering = 'unavailable' AND NEW.tool_calls IS NOT NULL)
  OR (NEW.tool_calls_metering <> 'unavailable' AND NEW.tool_calls IS NULL)
  OR (NEW.tokens_metering = 'unavailable' AND NEW.tokens IS NOT NULL)
  OR (NEW.tokens_metering <> 'unavailable' AND NEW.tokens IS NULL)
  OR NOT EXISTS (
    SELECT 1 FROM feature_run_budget_reservations AS reservation
    WHERE reservation.id = NEW.reservation_id
      AND reservation.run_id = NEW.run_id
      AND reservation.phase = NEW.phase
      AND reservation.contract_digest IS NOT NULL
  )
  OR NEW.sequence <> COALESCE((
    SELECT MAX(previous.sequence) + 1
    FROM feature_run_budget_observations AS previous
    WHERE previous.reservation_id = NEW.reservation_id
  ), 1)
)
BEGIN
  SELECT RAISE(ABORT, 'invalid FeatureRun budget observation sequence or ownership');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_observations_no_update
BEFORE UPDATE ON feature_run_budget_observations
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget observations are append-only');
END;

CREATE TRIGGER IF NOT EXISTS feature_run_budget_observations_no_delete
BEFORE DELETE ON feature_run_budget_observations
BEGIN
  SELECT RAISE(ABORT, 'FeatureRun budget observations are append-only');
END;
"#,
    )
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_accepted_legacy_review_chains_once(conn: &Connection) -> rusqlite::Result<()> {
    let completed = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [LEGACY_REVIEW_SETTLEMENT_MARKER],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|version| version == LEGACY_REVIEW_SETTLEMENT_VERSION);
    if completed {
        return Ok(());
    }
    settle_accepted_legacy_review_chains(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
        [
            LEGACY_REVIEW_SETTLEMENT_MARKER,
            LEGACY_REVIEW_SETTLEMENT_VERSION,
        ],
    )?;
    Ok(())
}

/// Settles only the graph-era chains whose own immutable logs prove that their
/// final independent review accepted the completed leaf fix. The migration is
/// deliberately a status projection: historical items, links, logs, results,
/// and review findings remain in place, and no legacy row is promoted into a
/// FeatureRun or ReviewGate.
fn settle_accepted_legacy_review_chains(conn: &Connection) -> rusqlite::Result<()> {
    if !table_exists(conn, "items")?
        || !table_exists(conn, "links")?
        || !table_exists(conn, "logs")?
        || !table_exists(conn, "events")?
    {
        return Ok(());
    }

    // Start only from currently stranded targets and walk their historical
    // review -> fix edges forward. This avoids scanning every accepted review
    // ever written. Rust still revalidates every evidence and graph edge
    // before mutation; this query is only a bounded candidate prefilter.
    let candidates = {
        let mut stmt = conn.prepare(
            r#"
WITH RECURSIVE reachable_target(target_id) AS (
  SELECT DISTINCT stranded.id
  FROM items AS stranded
  JOIN links AS reviewed_by
    ON reviewed_by.to_item = stranded.id AND reviewed_by.kind = 'reviews'
  WHERE stranded.status = 'in_review' AND stranded.work_type <> 'review'
  UNION
  SELECT child_fix.id
  FROM reachable_target AS current
  JOIN links AS review_link
    ON review_link.to_item = current.target_id AND review_link.kind = 'reviews'
  JOIN items AS reviewer
    ON reviewer.id = review_link.from_item
   AND reviewer.work_type = 'review'
   AND reviewer.status = 'closed'
  JOIN links AS blocks_child
    ON blocks_child.from_item = reviewer.id AND blocks_child.kind = 'blocks'
  JOIN items AS child_fix
    ON child_fix.id = blocks_child.to_item AND child_fix.work_type = 'fix'
)
SELECT DISTINCT reviewer.id, reviewed.id
FROM reachable_target AS reachable
JOIN items AS reviewed
  ON reviewed.id = reachable.target_id
 AND reviewed.work_type = 'fix'
 AND reviewed.status = 'closed'
JOIN links AS review_link
  ON review_link.to_item = reviewed.id AND review_link.kind = 'reviews'
JOIN items AS reviewer
  ON reviewer.id = review_link.from_item
 AND reviewer.work_type = 'review'
 AND reviewer.status = 'closed'
ORDER BY reviewer.id, reviewed.id
"#,
        )?;
        stmt.query_map([], |row| {
            Ok(LegacyReviewStep {
                review_id: row.get(0)?,
                target_id: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    for leaf in candidates {
        if !latest_review_evidence_matches(conn, &leaf.review_id, true)?
            || !review_step_is_unique_current(conn, &leaf, None)?
            || !target_has_completion_log(conn, &leaf.target_id)?
            || item_status(conn, &leaf.target_id)?.as_deref() != Some("closed")
        {
            continue;
        }

        let mut targets = vec![leaf.target_id.clone()];
        let mut seen = HashSet::from([leaf.target_id]);
        let mut current = targets[0].clone();
        let root_id = loop {
            let work_type = item_work_type(conn, &current)?.unwrap_or_default();
            let predecessors = legacy_predecessors(conn, &current)?;
            if work_type != "fix" {
                if predecessors.is_empty() {
                    break current;
                }
                targets.clear();
                break String::new();
            }
            if predecessors.len() != 1 {
                targets.clear();
                break String::new();
            }
            let predecessor = &predecessors[0];
            if !latest_review_evidence_matches(conn, &predecessor.review_id, false)?
                || !review_step_is_unique_current(conn, predecessor, Some(&current))?
                || !target_has_completion_log(conn, &predecessor.target_id)?
                || !matches!(
                    item_status(conn, &predecessor.target_id)?.as_deref(),
                    Some("in_review" | "closed")
                )
                || !seen.insert(predecessor.target_id.clone())
            {
                targets.clear();
                break String::new();
            }
            current = predecessor.target_id.clone();
            targets.push(current.clone());
        };
        if targets.is_empty() || root_id.is_empty() || project_has_open_review_gate(conn, &root_id)?
        {
            continue;
        }

        // Only the stranded ancestors change. Already terminal leaves and
        // historical review decisions retain their original timestamps.
        for target_id in &targets {
            conn.execute(
                "UPDATE items SET status = 'closed', completed_at = COALESCE(completed_at, datetime('now')), updated_at = datetime('now') WHERE id = ?1 AND status = 'in_review'",
                [target_id],
            )?;
        }

        // Pre-created follow-up reviews with no review verdict are obsolete
        // after the accepted leaf. They remain queryable but cannot be picked.
        for target_id in &targets {
            conn.execute(
                r#"
UPDATE items
SET status = 'cancelled', updated_at = datetime('now')
WHERE id IN (
  SELECT placeholder.id
  FROM items AS placeholder
  JOIN links AS review_link
    ON review_link.from_item = placeholder.id
   AND review_link.to_item = ?1
   AND review_link.kind = 'reviews'
  WHERE placeholder.work_type = 'review'
    AND placeholder.status IN ('pending','ready','blocked')
    AND NOT EXISTS (
      SELECT 1 FROM logs WHERE logs.item_id = placeholder.id
    )
    AND NOT EXISTS (
      SELECT 1 FROM events
      WHERE events.item_id = placeholder.id
        AND events.event_type NOT IN ('item_created','link_added')
    )
    AND NOT EXISTS (
      SELECT 1 FROM links
      WHERE links.from_item = placeholder.id AND links.kind = 'blocks'
    )
)
"#,
                [target_id],
            )?;
        }
    }
    Ok(())
}

/// A selected legacy review is authoritative only when it is the sole review
/// with durable activity for its target and owns exactly the expected fix
/// branch. Empty pre-created siblings are tolerated so the accepted chain can
/// cancel them later; any worked, running, or competing sibling fails closed.
fn review_step_is_unique_current(
    conn: &Connection,
    selected: &LegacyReviewStep,
    expected_child: Option<&str>,
) -> rusqlite::Result<bool> {
    let outgoing_children = {
        let mut stmt = conn.prepare(
            "SELECT to_item FROM links WHERE from_item = ?1 AND kind = 'blocks' ORDER BY to_item",
        )?;
        stmt.query_map([&selected.review_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    match expected_child {
        Some(child) if outgoing_children.as_slice() != [child] => return Ok(false),
        None if !outgoing_children.is_empty() => return Ok(false),
        _ => {}
    }

    let siblings = {
        let mut stmt = conn.prepare(
            r#"
SELECT sibling.id, sibling.status
FROM links AS review_link
JOIN items AS sibling
  ON sibling.id = review_link.from_item AND sibling.work_type = 'review'
WHERE review_link.to_item = ?1 AND review_link.kind = 'reviews'
ORDER BY sibling.id
"#,
        )?;
        stmt.query_map([&selected.target_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if siblings
        .iter()
        .filter(|(id, _)| id == &selected.review_id)
        .count()
        != 1
    {
        return Ok(false);
    }
    for (sibling_id, status) in siblings {
        if sibling_id == selected.review_id {
            continue;
        }
        let has_logs: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM logs WHERE item_id = ?1)",
            [&sibling_id],
            |row| row.get(0),
        )?;
        let has_structured_activity: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE item_id = ?1 AND event_type NOT IN ('item_created','link_added'))",
            [&sibling_id],
            |row| row.get(0),
        )?;
        let owns_workflow_branch: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM links WHERE from_item = ?1 AND kind = 'blocks')",
            [&sibling_id],
            |row| row.get(0),
        )?;
        if !matches!(
            status.as_str(),
            "pending" | "ready" | "blocked" | "cancelled"
        ) || has_logs
            || has_structured_activity
            || owns_workflow_branch
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn latest_review_evidence_matches(
    conn: &Connection,
    review_id: &str,
    accepted: bool,
) -> rusqlite::Result<bool> {
    let event = conn
        .query_row(
            "SELECT payload FROM events WHERE item_id = ?1 AND event_type = 'review_closed' ORDER BY timestamp DESC, id DESC LIMIT 1",
            [review_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let findings = conn
        .query_row(
            "SELECT review_findings FROM logs WHERE item_id = ?1 AND kind = 'review' ORDER BY created_at DESC, id DESC LIMIT 1",
            [review_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(event) = event else {
        return Ok(false);
    };
    let findings = findings
        .flatten()
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<serde_json::Value>>(raw).ok());
    Ok(if accepted {
        event["verdict"] == "complete"
            && event["review_mode"] == "independent"
            && findings.as_ref().is_some_and(Vec::is_empty)
    } else {
        event["verdict"] == "not-complete"
            && event["review_mode"] == "independent"
            && findings.as_ref().is_some_and(|values| !values.is_empty())
    })
}

fn target_has_completion_log(conn: &Connection, item_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM logs WHERE item_id = ?1 AND kind = 'completion' AND length(trim(summary)) > 0)",
        [item_id],
        |row| row.get(0),
    )
}

fn item_status(conn: &Connection, item_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT status FROM items WHERE id = ?1", [item_id], |row| {
        row.get(0)
    })
    .optional()
}

fn item_work_type(conn: &Connection, item_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT work_type FROM items WHERE id = ?1",
        [item_id],
        |row| row.get(0),
    )
    .optional()
}

fn legacy_predecessors(
    conn: &Connection,
    child_target_id: &str,
) -> rusqlite::Result<Vec<LegacyReviewStep>> {
    let mut stmt = conn.prepare(
        r#"
SELECT reviewer.id, parent_target.id
FROM links AS blocks_child
JOIN items AS reviewer
  ON reviewer.id = blocks_child.from_item
 AND reviewer.work_type = 'review'
 AND reviewer.status = 'closed'
JOIN links AS reviews_parent
  ON reviews_parent.from_item = reviewer.id
 AND reviews_parent.kind = 'reviews'
JOIN items AS parent_target ON parent_target.id = reviews_parent.to_item
WHERE blocks_child.to_item = ?1 AND blocks_child.kind = 'blocks'
ORDER BY reviewer.id, parent_target.id
"#,
    )?;
    stmt.query_map([child_target_id], |row| {
        Ok(LegacyReviewStep {
            review_id: row.get(0)?,
            target_id: row.get(1)?,
        })
    })?
    .collect()
}

fn project_has_open_review_gate(conn: &Connection, root_item_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        r#"
SELECT EXISTS(
  SELECT 1
  FROM review_gates AS gate
  JOIN feature_runs AS run ON run.id = gate.run_id
  JOIN items AS root ON root.id = ?1 AND root.project_id = run.project_id
  WHERE gate.status NOT IN ('accepted','cancelled')
)
"#,
        [root_item_id],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn legacy_database() -> Connection {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch(
            r#"
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE items(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  status TEXT NOT NULL,
  work_type TEXT NOT NULL,
  completed_at TEXT,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE links(
  from_item TEXT NOT NULL,
  to_item TEXT NOT NULL,
  kind TEXT NOT NULL,
  UNIQUE(from_item, to_item, kind)
);
CREATE TABLE logs(
  id TEXT PRIMARY KEY,
  item_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  summary TEXT NOT NULL,
  review_findings TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id TEXT,
  event_type TEXT NOT NULL,
  payload TEXT,
  timestamp TEXT NOT NULL DEFAULT (datetime('now'))
);
"#,
        )
        .expect("legacy tables");
        conn
    }

    fn insert_accepted_chain(conn: &Connection, suffix: &str) {
        let sql = format!(
            r#"
INSERT INTO items(id, project_id, status, work_type) VALUES
  ('root-{suffix}', 'project-{suffix}', 'in_review', 'code'),
  ('review-root-{suffix}', 'project-{suffix}', 'closed', 'review'),
  ('ancestor-fix-{suffix}', 'project-{suffix}', 'in_review', 'fix'),
  ('review-fix-{suffix}', 'project-{suffix}', 'closed', 'review'),
  ('leaf-fix-{suffix}', 'project-{suffix}', 'closed', 'fix'),
  ('accepted-{suffix}', 'project-{suffix}', 'closed', 'review'),
  ('placeholder-root-{suffix}', 'project-{suffix}', 'pending', 'review'),
  ('placeholder-fix-{suffix}', 'project-{suffix}', 'ready', 'review');
INSERT INTO links(from_item, to_item, kind) VALUES
  ('review-root-{suffix}', 'root-{suffix}', 'reviews'),
  ('review-root-{suffix}', 'ancestor-fix-{suffix}', 'blocks'),
  ('review-fix-{suffix}', 'ancestor-fix-{suffix}', 'reviews'),
  ('review-fix-{suffix}', 'leaf-fix-{suffix}', 'blocks'),
  ('accepted-{suffix}', 'leaf-fix-{suffix}', 'reviews'),
  ('ancestor-fix-{suffix}', 'placeholder-root-{suffix}', 'blocks'),
  ('placeholder-root-{suffix}', 'root-{suffix}', 'reviews'),
  ('leaf-fix-{suffix}', 'placeholder-fix-{suffix}', 'blocks'),
  ('placeholder-fix-{suffix}', 'ancestor-fix-{suffix}', 'reviews');
INSERT INTO logs(id, item_id, kind, summary, review_findings) VALUES
  ('root-log-{suffix}', 'root-{suffix}', 'completion', 'implemented', NULL),
  ('review-root-log-{suffix}', 'review-root-{suffix}', 'review', 'historical review', '["root finding"]'),
  ('ancestor-fix-log-{suffix}', 'ancestor-fix-{suffix}', 'completion', 'fixed root finding', NULL),
  ('review-fix-log-{suffix}', 'review-fix-{suffix}', 'review', 'historical review', '["fix finding"]'),
  ('leaf-fix-log-{suffix}', 'leaf-fix-{suffix}', 'completion', 'fixed remaining finding', NULL),
  ('accepted-log-{suffix}', 'accepted-{suffix}', 'review', 'review verdict: complete (reviewer: independent)', '[]');
INSERT INTO events(item_id, event_type, payload) VALUES
  ('review-root-{suffix}', 'review_closed', '{{"verdict":"not-complete","review_mode":"independent","created":2}}'),
  ('review-fix-{suffix}', 'review_closed', '{{"verdict":"not-complete","review_mode":"independent","created":2}}'),
  ('accepted-{suffix}', 'review_closed', '{{"verdict":"complete","review_mode":"independent","created":0}}');
"#
        );
        conn.execute_batch(&sql).expect("accepted legacy chain");
    }

    fn status(conn: &Connection, id: &str) -> String {
        conn.query_row("SELECT status FROM items WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("item status")
    }

    fn object_count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("object count")
    }

    fn assert_chain_unsettled(conn: &Connection, suffix: &str) {
        assert_eq!(status(conn, &format!("root-{suffix}")), "in_review");
        assert_eq!(status(conn, &format!("ancestor-fix-{suffix}")), "in_review");
        assert_eq!(
            status(conn, &format!("placeholder-root-{suffix}")),
            "pending"
        );
        assert_eq!(status(conn, &format!("placeholder-fix-{suffix}")), "ready");
    }

    #[test]
    fn failed_upgrade_rolls_back_every_new_canonical_object() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch("CREATE TABLE execution_batches(id TEXT PRIMARY KEY)")
            .expect("conflicting historical table");
        assert!(ensure_execution_run_schema(&conn).is_err());
        let feature_runs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'feature_runs'",
                [],
                |row| row.get(0),
            )
            .expect("schema inspection");
        assert_eq!(feature_runs, 0);
        let historical_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'execution_batches'",
                [],
                |row| row.get(0),
            )
            .expect("historical table inspection");
        assert_eq!(historical_table, 1);
    }

    #[test]
    fn accepted_legacy_chain_settles_ancestors_and_cancels_empty_follow_up() {
        let conn = legacy_database();
        insert_accepted_chain(&conn, "accepted");
        let links_before = object_count(&conn, "links");
        let logs_before = object_count(&conn, "logs");

        ensure_execution_run_schema(&conn).expect("upgrade");

        assert_eq!(status(&conn, "root-accepted"), "closed");
        assert_eq!(status(&conn, "ancestor-fix-accepted"), "closed");
        assert_eq!(status(&conn, "leaf-fix-accepted"), "closed");
        assert_eq!(status(&conn, "review-root-accepted"), "closed");
        assert_eq!(status(&conn, "review-fix-accepted"), "closed");
        assert_eq!(status(&conn, "accepted-accepted"), "closed");
        assert_eq!(status(&conn, "placeholder-root-accepted"), "cancelled");
        assert_eq!(status(&conn, "placeholder-fix-accepted"), "cancelled");
        assert_eq!(object_count(&conn, "links"), links_before);
        assert_eq!(object_count(&conn, "logs"), logs_before);
        assert_eq!(object_count(&conn, "feature_runs"), 0);
        assert_eq!(object_count(&conn, "review_gates"), 0);
    }

    #[test]
    fn open_or_failed_legacy_chain_never_infers_acceptance_from_status() {
        let conn = legacy_database();
        insert_accepted_chain(&conn, "open");
        conn.execute("DELETE FROM logs WHERE id = 'accepted-log-open'", [])
            .expect("remove accepted verdict");
        insert_accepted_chain(&conn, "failed");
        conn.execute(
            "UPDATE items SET status = 'failed' WHERE id = 'leaf-fix-failed'",
            [],
        )
        .expect("fail leaf fix");

        ensure_execution_run_schema(&conn).expect("upgrade");

        assert_eq!(status(&conn, "root-open"), "in_review");
        assert_eq!(status(&conn, "placeholder-root-open"), "pending");
        assert_eq!(status(&conn, "placeholder-fix-open"), "ready");
        assert_eq!(status(&conn, "root-failed"), "in_review");
        assert_eq!(status(&conn, "ancestor-fix-failed"), "in_review");
        assert_eq!(status(&conn, "leaf-fix-failed"), "failed");
        assert_eq!(status(&conn, "placeholder-fix-failed"), "ready");
    }

    #[test]
    fn older_acceptance_loses_to_later_independent_changes_requested() {
        let conn = legacy_database();
        insert_accepted_chain(&conn, "superseded");
        conn.execute_batch(
            r#"
INSERT INTO logs(id, item_id, kind, summary, review_findings, created_at) VALUES
  ('later-review-log', 'accepted-superseded', 'review', 'later review', '["unresolved"]', datetime('now', '+1 second'));
INSERT INTO events(item_id, event_type, payload, timestamp) VALUES
  ('accepted-superseded', 'review_closed', '{"verdict":"not-complete","review_mode":"independent","created":2}', datetime('now', '+1 second'));
"#,
        )
        .expect("later changes requested");

        ensure_execution_run_schema(&conn).expect("upgrade");

        assert_chain_unsettled(&conn, "superseded");
        assert_eq!(status(&conn, "accepted-superseded"), "closed");
    }

    #[test]
    fn picked_running_or_logged_sibling_review_blocks_settlement_and_cancellation() {
        let conn = legacy_database();
        for suffix in ["picked", "running", "logged"] {
            insert_accepted_chain(&conn, suffix);
        }
        conn.execute_batch(
            r#"
INSERT INTO items(id, project_id, status, work_type) VALUES
  ('sibling-picked', 'project-picked', 'picked', 'review'),
  ('sibling-running', 'project-running', 'running', 'review');
INSERT INTO links(from_item, to_item, kind) VALUES
  ('sibling-picked', 'leaf-fix-picked', 'reviews'),
  ('sibling-running', 'leaf-fix-running', 'reviews');
INSERT INTO logs(id, item_id, kind, summary, review_findings) VALUES
  ('placeholder-activity-log', 'placeholder-fix-logged', 'review_feedback', 'unresolved feedback', '["unresolved"]');
"#,
        )
        .expect("active siblings");

        ensure_execution_run_schema(&conn).expect("upgrade");

        for suffix in ["picked", "running", "logged"] {
            assert_chain_unsettled(&conn, suffix);
        }
        assert_eq!(status(&conn, "sibling-picked"), "picked");
        assert_eq!(status(&conn, "sibling-running"), "running");
        assert_eq!(status(&conn, "placeholder-fix-logged"), "ready");
    }

    #[test]
    fn sibling_owned_fix_branch_blocks_settlement_and_preserves_orphan_fix() {
        let conn = legacy_database();
        insert_accepted_chain(&conn, "sibling-branch");
        conn.execute_batch(
            r#"
INSERT INTO items(id, project_id, status, work_type)
VALUES ('orphan-alternate-fix', 'project-sibling-branch', 'ready', 'fix');
INSERT INTO links(from_item, to_item, kind)
VALUES ('placeholder-fix-sibling-branch', 'orphan-alternate-fix', 'blocks');
"#,
        )
        .expect("sibling-owned alternate branch");

        ensure_execution_run_schema(&conn).expect("upgrade");

        assert_chain_unsettled(&conn, "sibling-branch");
        assert_eq!(status(&conn, "placeholder-fix-sibling-branch"), "ready");
        assert_eq!(status(&conn, "orphan-alternate-fix"), "ready");
    }

    #[test]
    fn ambiguous_accepted_sibling_or_fix_branch_blocks_settlement() {
        let conn = legacy_database();
        insert_accepted_chain(&conn, "siblings");
        insert_accepted_chain(&conn, "branch");
        conn.execute_batch(
            r#"
INSERT INTO items(id, project_id, status, work_type) VALUES
  ('accepted-sibling', 'project-siblings', 'closed', 'review'),
  ('alternate-leaf', 'project-branch', 'closed', 'fix');
INSERT INTO links(from_item, to_item, kind) VALUES
  ('accepted-sibling', 'leaf-fix-siblings', 'reviews'),
  ('review-fix-branch', 'alternate-leaf', 'blocks');
INSERT INTO logs(id, item_id, kind, summary, review_findings) VALUES
  ('accepted-sibling-log', 'accepted-sibling', 'review', 'competing acceptance', '[]'),
  ('alternate-leaf-log', 'alternate-leaf', 'completion', 'alternate fix', NULL);
INSERT INTO events(item_id, event_type, payload) VALUES
  ('accepted-sibling', 'review_closed', '{"verdict":"complete","review_mode":"independent","created":0}');
"#,
        )
        .expect("ambiguous histories");

        ensure_execution_run_schema(&conn).expect("upgrade");

        assert_chain_unsettled(&conn, "siblings");
        assert_chain_unsettled(&conn, "branch");
    }

    #[test]
    fn open_canonical_review_gate_blocks_legacy_settlement() {
        let conn = legacy_database();
        ensure_execution_run_schema(&conn).expect("canonical schema");
        conn.execute(
            "DELETE FROM meta WHERE key = ?1",
            [LEGACY_REVIEW_SETTLEMENT_MARKER],
        )
        .expect("re-arm migration fixture");
        insert_accepted_chain(&conn, "gated");
        conn.execute_batch(
            r#"
INSERT INTO feature_runs(id, project_id, plan_id, status, phase, policy_digest, source_revision)
VALUES ('run-gated', 'project-gated', 'plan-gated', 'active', 'final_review', 'sha256:policy', 'source-a');
INSERT INTO review_gates(id, run_id, scope_kind, scope_id, kind, status, responsible_maker_id, source_revision)
VALUES ('gate-gated', 'run-gated', 'outcome', 'root-gated', 'final_product', 'pending', 'maker-a', 'source-a');
"#,
        )
        .expect("open canonical gate");

        ensure_execution_run_schema(&conn).expect("restart upgrade");

        assert_chain_unsettled(&conn, "gated");
        assert_eq!(status(&conn, "accepted-gated"), "closed");
        assert_eq!(object_count(&conn, "review_gates"), 1);
    }

    #[test]
    fn accepted_legacy_chain_upgrade_is_restart_idempotent() {
        let conn = legacy_database();
        conn.execute_batch(
            r#"
WITH RECURSIVE sequence(value) AS (
  SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 2000
)
INSERT INTO items(id, project_id, status, work_type)
SELECT printf('irrelevant-target-%04d', value), 'project-restart', 'closed', 'code'
FROM sequence;
WITH RECURSIVE sequence(value) AS (
  SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 2000
)
INSERT INTO items(id, project_id, status, work_type)
SELECT printf('irrelevant-review-%04d', value), 'project-restart', 'closed', 'review'
FROM sequence;
WITH RECURSIVE sequence(value) AS (
  SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < 2000
)
INSERT INTO links(from_item, to_item, kind)
SELECT printf('irrelevant-review-%04d', value), printf('irrelevant-target-%04d', value), 'reviews'
FROM sequence;
"#,
        )
        .expect("irrelevant historical reviews");
        insert_accepted_chain(&conn, "restart");
        let first_started = Instant::now();
        ensure_execution_run_schema(&conn).expect("first upgrade");
        let first_elapsed = first_started.elapsed();
        let first: (String, String) = conn
            .query_row(
                "SELECT completed_at, updated_at FROM items WHERE id = 'root-restart'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("first timestamps");

        let restart_started = Instant::now();
        ensure_execution_run_schema(&conn).expect("restart upgrade");
        let restart_elapsed = restart_started.elapsed();
        let second: (String, String) = conn
            .query_row(
                "SELECT completed_at, updated_at FROM items WHERE id = 'root-restart'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("second timestamps");

        assert_eq!(second, first);
        assert_eq!(status(&conn, "placeholder-root-restart"), "cancelled");
        assert_eq!(status(&conn, "placeholder-fix-restart"), "cancelled");
        assert_eq!(object_count(&conn, "feature_runs"), 0);
        assert_eq!(object_count(&conn, "review_gates"), 0);
        let marker: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [LEGACY_REVIEW_SETTLEMENT_MARKER],
                |row| row.get(0),
            )
            .expect("migration marker");
        assert_eq!(marker, LEGACY_REVIEW_SETTLEMENT_VERSION);
        assert_eq!(status(&conn, "irrelevant-review-2000"), "closed");
        assert!(first_elapsed < Duration::from_secs(2), "{first_elapsed:?}");
        assert!(
            restart_elapsed < Duration::from_millis(500),
            "{restart_elapsed:?}"
        );
    }

    #[test]
    fn accepted_legacy_chain_upgrade_rolls_back_status_projection_atomically() {
        let conn = legacy_database();
        insert_accepted_chain(&conn, "rollback");
        conn.execute_batch(
            r#"
CREATE TRIGGER reject_root_settlement
BEFORE UPDATE OF status ON items
WHEN OLD.id = 'root-rollback' AND NEW.status = 'closed'
BEGIN
  SELECT RAISE(ABORT, 'injected settlement failure');
END;
"#,
        )
        .expect("failure trigger");

        let error = ensure_execution_run_schema(&conn).expect_err("upgrade must fail");
        assert!(error.to_string().contains("injected settlement failure"));
        assert_eq!(status(&conn, "root-rollback"), "in_review");
        assert_eq!(status(&conn, "ancestor-fix-rollback"), "in_review");
        assert_eq!(status(&conn, "placeholder-root-rollback"), "pending");
        assert_eq!(status(&conn, "placeholder-fix-rollback"), "ready");
        let canonical_objects: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('feature_runs','review_gates')",
                [],
                |row| row.get(0),
            )
            .expect("canonical objects");
        assert_eq!(canonical_objects, 0);
        let marker: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meta WHERE key = ?1)",
                [LEGACY_REVIEW_SETTLEMENT_MARKER],
                |row| row.get(0),
            )
            .expect("migration marker state");
        assert!(!marker);
    }
}
