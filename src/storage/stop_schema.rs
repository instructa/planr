use anyhow::Result;
use rusqlite::Connection;

const STOP_SCHEMA_VERSION: &str = "2";

pub fn ensure_stop_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS stop_active_bindings(
  key TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0),
  plan_id TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','cancelled','settled')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(plan_id) REFERENCES plans(id),
  UNIQUE(project_id, session_id)
);
CREATE TABLE IF NOT EXISTS stop_enforcement_state(
  key TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0),
  scope_kind TEXT NOT NULL DEFAULT 'plan' CHECK(scope_kind = 'plan'),
  scope_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  total_count INTEGER NOT NULL CHECK(total_count >= 0),
  same_count INTEGER NOT NULL CHECK(same_count >= 0),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(plan_id) REFERENCES plans(id),
  CHECK(scope_id = plan_id)
);
"#,
    )?;
    migrate_stop_schema(conn)?;
    install_stop_integrity_guards(conn)?;
    Ok(())
}

fn migrate_stop_schema(conn: &Connection) -> Result<()> {
    let version = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'stop_schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    if version.as_deref() == Some(STOP_SCHEMA_VERSION) {
        return Ok(());
    }
    rebuild_stop_active_bindings(conn)?;
    rebuild_stop_enforcement_state(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('stop_schema_version', ?1)",
        [STOP_SCHEMA_VERSION],
    )?;
    Ok(())
}

fn rebuild_stop_active_bindings(conn: &Connection) -> Result<()> {
    let item_filter = if table_has_column(conn, "stop_active_bindings", "item_id")? {
        "AND (b.item_id IS NULL OR b.item_id = '')"
    } else {
        ""
    };
    conn.execute_batch(&format!(
        r#"
ALTER TABLE stop_active_bindings RENAME TO stop_active_bindings_old;
CREATE TABLE stop_active_bindings(
  key TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0),
  plan_id TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','cancelled','settled')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(plan_id) REFERENCES plans(id),
  UNIQUE(project_id, session_id)
);
INSERT OR IGNORE INTO stop_active_bindings(key, project_id, session_id, plan_id, status, updated_at)
SELECT
  'session:' || b.project_id || ':' || b.session_id,
  b.project_id,
  b.session_id,
  b.plan_id,
  CASE WHEN b.status IN ('active','cancelled','settled') THEN b.status ELSE 'cancelled' END,
  COALESCE(b.updated_at, datetime('now'))
FROM stop_active_bindings_old b
JOIN projects p ON p.id = b.project_id
JOIN plans pl ON pl.id = b.plan_id AND pl.project_id = b.project_id
WHERE b.session_id IS NOT NULL
  AND trim(b.session_id) != ''
  AND b.plan_id IS NOT NULL
  {item_filter};
DROP TABLE stop_active_bindings_old;
"#
    ))?;
    Ok(())
}

fn rebuild_stop_enforcement_state(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
DROP INDEX IF EXISTS idx_stop_enforcement_state_scope;
ALTER TABLE stop_enforcement_state RENAME TO stop_enforcement_state_old;
CREATE TABLE stop_enforcement_state(
  key TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0),
  scope_kind TEXT NOT NULL DEFAULT 'plan' CHECK(scope_kind = 'plan'),
  scope_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  total_count INTEGER NOT NULL CHECK(total_count >= 0),
  same_count INTEGER NOT NULL CHECK(same_count >= 0),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(plan_id) REFERENCES plans(id),
  CHECK(scope_id = plan_id)
);
INSERT OR IGNORE INTO stop_enforcement_state(
  key, project_id, session_id, scope_kind, scope_id, plan_id,
  fingerprint, total_count, same_count, updated_at
)
SELECT
  s.project_id || ':' || s.session_id || ':plan:' || COALESCE(s.plan_id, s.scope_id),
  s.project_id,
  s.session_id,
  'plan',
  COALESCE(s.plan_id, s.scope_id),
  COALESCE(s.plan_id, s.scope_id),
  s.fingerprint,
  CASE WHEN s.total_count >= 0 THEN s.total_count ELSE 0 END,
  CASE WHEN s.same_count >= 0 THEN s.same_count ELSE 0 END,
  COALESCE(s.updated_at, datetime('now'))
FROM stop_enforcement_state_old s
JOIN projects p ON p.id = s.project_id
JOIN plans pl ON pl.id = COALESCE(s.plan_id, s.scope_id) AND pl.project_id = s.project_id
WHERE s.session_id IS NOT NULL
  AND trim(s.session_id) != ''
  AND COALESCE(s.scope_kind, 'plan') = 'plan'
  AND COALESCE(s.plan_id, s.scope_id) IS NOT NULL
  AND COALESCE(s.plan_id, s.scope_id) = s.scope_id;
DROP TABLE stop_enforcement_state_old;
CREATE INDEX IF NOT EXISTS idx_stop_enforcement_state_scope
  ON stop_enforcement_state(project_id, session_id, scope_kind, scope_id);
"#,
    )?;
    Ok(())
}

fn install_stop_integrity_guards(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TRIGGER IF NOT EXISTS stop_active_bindings_project_plan_insert
BEFORE INSERT ON stop_active_bindings
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_active_bindings plan must belong to project');
END;
CREATE TRIGGER IF NOT EXISTS stop_active_bindings_project_plan_update
BEFORE UPDATE OF project_id, plan_id ON stop_active_bindings
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_active_bindings plan must belong to project');
END;
CREATE TRIGGER IF NOT EXISTS stop_enforcement_state_project_plan_insert
BEFORE INSERT ON stop_enforcement_state
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_enforcement_state plan must belong to project');
END;
CREATE TRIGGER IF NOT EXISTS stop_enforcement_state_project_plan_update
BEFORE UPDATE OF project_id, plan_id ON stop_enforcement_state
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_enforcement_state plan must belong to project');
END;
CREATE INDEX IF NOT EXISTS idx_stop_enforcement_state_scope
  ON stop_enforcement_state(project_id, session_id, scope_kind, scope_id);
"#,
    )?;
    Ok(())
}

fn table_has_column(conn: &Connection, table: &str, name: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == name {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use crate::storage::ensure_schema;
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn stop_schema_rebuilds_legacy_rows_to_exact_session_plan_scope() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("legacy-v1.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            seed_exact_stop_schema_v1(&conn);
        }

        {
            let conn = Connection::open(&path).unwrap();
            ensure_schema(&conn).unwrap();
            assert_migrated_stop_schema_v2(&conn);
            ensure_schema(&conn).unwrap();
            assert_migrated_stop_schema_v2(&conn);
        }

        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        assert_migrated_stop_schema_v2(&conn);
    }

    fn seed_exact_stop_schema_v1(conn: &Connection) {
        conn.execute_batch(
            r#"
PRAGMA foreign_keys = ON;
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
INSERT INTO meta(key, value) VALUES ('stop_schema_version', '1');
CREATE TABLE projects(id TEXT PRIMARY KEY, name TEXT NOT NULL, root_path TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE plans(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, stage TEXT NOT NULL, path TEXT NOT NULL, title TEXT NOT NULL, slug TEXT NOT NULL, parse_status TEXT NOT NULL, content_hash TEXT NOT NULL, archived INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, FOREIGN KEY(project_id) REFERENCES projects(id));
CREATE TABLE stop_active_bindings(
  key TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0),
  plan_id TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','cancelled','settled')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(plan_id) REFERENCES plans(id),
  UNIQUE(project_id, session_id)
);
CREATE TABLE stop_enforcement_state(
  key TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL DEFAULT 'plan' CHECK(scope_kind = 'plan'),
  scope_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  total_count INTEGER NOT NULL CHECK(total_count >= 0),
  same_count INTEGER NOT NULL CHECK(same_count >= 0),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY(project_id) REFERENCES projects(id),
  FOREIGN KEY(plan_id) REFERENCES plans(id),
  CHECK(scope_id = plan_id)
);
CREATE TRIGGER IF NOT EXISTS stop_active_bindings_project_plan_insert
BEFORE INSERT ON stop_active_bindings
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_active_bindings plan must belong to project');
END;
CREATE TRIGGER IF NOT EXISTS stop_active_bindings_project_plan_update
BEFORE UPDATE OF project_id, plan_id ON stop_active_bindings
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_active_bindings plan must belong to project');
END;
CREATE TRIGGER IF NOT EXISTS stop_enforcement_state_project_plan_insert
BEFORE INSERT ON stop_enforcement_state
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_enforcement_state plan must belong to project');
END;
CREATE TRIGGER IF NOT EXISTS stop_enforcement_state_project_plan_update
BEFORE UPDATE OF project_id, plan_id ON stop_enforcement_state
FOR EACH ROW
WHEN NOT EXISTS (
  SELECT 1 FROM plans
  WHERE id = NEW.plan_id AND project_id = NEW.project_id
)
BEGIN
  SELECT RAISE(ABORT, 'stop_enforcement_state plan must belong to project');
END;
CREATE INDEX IF NOT EXISTS idx_stop_enforcement_state_scope
  ON stop_enforcement_state(project_id, session_id, scope_kind, scope_id);
INSERT INTO projects(id, name, root_path, created_at, updated_at) VALUES ('p', 'P', '.', datetime('now'), datetime('now'));
INSERT INTO projects(id, name, root_path, created_at, updated_at) VALUES ('other-p', 'Other', '.', datetime('now'), datetime('now'));
INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, archived, created_at, updated_at)
VALUES ('pln', 'p', 'build', 'plan.md', 'Plan', 'plan', 'ok', 'hash', 0, datetime('now'), datetime('now'));
INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, archived, created_at, updated_at)
VALUES ('other-pln', 'other-p', 'build', 'other.md', 'Other', 'other', 'ok', 'hash', 0, datetime('now'), datetime('now'));
INSERT INTO stop_active_bindings VALUES
  ('good', 'p', 'thread-a', 'pln', 'active', datetime('now'));
INSERT INTO stop_enforcement_state VALUES
  ('good-state', 'p', 'thread-a', 'plan', 'pln', 'pln', 'fp', 1, 1, datetime('now')),
  ('blank-session-state', 'p', '', 'plan', 'pln', 'pln', 'fp-blank', 2, 1, datetime('now'));
"#,
        )
        .unwrap();
        assert_exact_stop_schema_v1_contract(conn);
    }

    fn assert_exact_stop_schema_v1_contract(conn: &Connection) {
        assert_stop_scope_index_exists(conn);
        assert_stop_integrity_triggers_exist(conn);
        assert_stop_table_sql_has_nonblank_session_check(conn, "stop_active_bindings");
        let enforcement_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'stop_enforcement_state'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !enforcement_sql
                .contains("session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0)"),
            "v1 fixture must preserve the historical missing nonblank enforcement session CHECK: {enforcement_sql}"
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('v1-blank-active', 'p', '', 'pln', 'active')",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('v1-bad-status', 'p', 'thread-bad-status', 'pln', 'paused')",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('v1-cross-active', 'p', 'thread-cross', 'other-pln', 'active')",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('v1-item-state', 'p', 'thread-item', 'item', 'pln', 'pln', 'fp', 1, 1)",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('v1-negative-state', 'p', 'thread-negative', 'plan', 'pln', 'pln', 'fp', -1, 1)",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('v1-mismatch-state', 'p', 'thread-mismatch', 'plan', 'other-pln', 'pln', 'fp', 1, 1)",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('v1-cross-state', 'p', 'thread-cross', 'plan', 'other-pln', 'other-pln', 'fp', 1, 1)",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE stop_active_bindings SET plan_id = 'other-pln' WHERE key = 'good'",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "UPDATE stop_enforcement_state SET plan_id = 'other-pln', scope_id = 'other-pln' WHERE key = 'good-state'",
                [],
            )
            .is_err()
        );
    }

    fn assert_migrated_stop_schema_v2(conn: &Connection) {
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'stop_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "2");

        let bindings: i64 = conn
            .query_row("SELECT COUNT(*) FROM stop_active_bindings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(bindings, 1);
        let plan_id: String = conn
            .query_row("SELECT plan_id FROM stop_active_bindings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(plan_id, "pln");
        let states: i64 = conn
            .query_row("SELECT COUNT(*) FROM stop_enforcement_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(states, 1);
        let retained: (String, String, String, i64, i64) = conn
            .query_row(
                "SELECT project_id, session_id, plan_id, total_count, same_count FROM stop_enforcement_state",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            retained,
            (
                "p".to_string(),
                "thread-a".to_string(),
                "pln".to_string(),
                1,
                1
            )
        );
        let rejected = conn.execute(
            "INSERT INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('bad', 'p', '', 'pln', 'active')",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "INSERT INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('cross', 'p', 'thread-cross', 'other-pln', 'active')",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('bad', 'p', 'thread-a', 'plan', 'other', 'pln', 'fp', 1, 1)",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('blank-state', 'p', '', 'plan', 'pln', 'pln', 'fp', 1, 1)",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('cross-state', 'p', 'thread-cross', 'plan', 'other-pln', 'other-pln', 'fp', 1, 1)",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "UPDATE stop_active_bindings SET plan_id = 'other-pln' WHERE key = 'session:p:thread-a'",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "UPDATE stop_enforcement_state SET session_id = '' WHERE key = 'p:thread-a:plan:pln'",
            [],
        );
        assert!(rejected.is_err());
        let rejected = conn.execute(
            "UPDATE stop_enforcement_state SET plan_id = 'other-pln', scope_id = 'other-pln' WHERE key = 'p:thread-a:plan:pln'",
            [],
        );
        assert!(rejected.is_err());
        assert_stop_table_sql_has_nonblank_session_check(conn, "stop_active_bindings");
        assert_stop_table_sql_has_nonblank_session_check(conn, "stop_enforcement_state");
        assert_stop_scope_index_exists(conn);
        assert_stop_integrity_triggers_exist(conn);
    }

    #[test]
    fn stop_schema_fresh_repeat_and_reopen_keeps_project_guards_and_index() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("planr.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            ensure_schema(&conn).unwrap();
            conn.execute(
                "INSERT INTO projects(id, name, root_path, created_at, updated_at) VALUES ('p', 'P', '.', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects(id, name, root_path, created_at, updated_at) VALUES ('other-p', 'Other', '.', datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, archived, created_at, updated_at)
                 VALUES ('pln', 'p', 'build', 'plan.md', 'Plan', 'plan', 'ok', 'hash', 0, datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO plans(id, project_id, stage, path, title, slug, parse_status, content_hash, archived, created_at, updated_at)
                 VALUES ('other-pln', 'other-p', 'build', 'other.md', 'Other', 'other', 'ok', 'hash', 0, datetime('now'), datetime('now'))",
                [],
            )
            .unwrap();
            assert_stop_schema_rejects_cross_project_and_blank(&conn);
            assert_stop_scope_index_exists(&conn);
            assert_stop_integrity_triggers_exist(&conn);
            ensure_schema(&conn).unwrap();
            assert_stop_schema_rejects_cross_project_and_blank(&conn);
            assert_stop_scope_index_exists(&conn);
            assert_stop_integrity_triggers_exist(&conn);
        }
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        assert_stop_schema_rejects_cross_project_and_blank(&conn);
        assert_stop_scope_index_exists(&conn);
        assert_stop_integrity_triggers_exist(&conn);
    }

    fn assert_stop_schema_rejects_cross_project_and_blank(conn: &Connection) {
        assert!(
            conn.execute(
                "INSERT OR IGNORE INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('fresh-good', 'p', 'thread-good', 'pln', 'active')",
                [],
            )
            .is_ok()
        );
        assert!(
            conn.execute(
                "INSERT OR REPLACE INTO stop_active_bindings(key, project_id, session_id, plan_id, status) VALUES ('fresh-cross', 'p', 'thread-cross', 'other-pln', 'active')",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('fresh-blank', 'p', '', 'plan', 'pln', 'pln', 'fp', 1, 1)",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO stop_enforcement_state(key, project_id, session_id, scope_kind, scope_id, plan_id, fingerprint, total_count, same_count) VALUES ('fresh-cross-state', 'p', 'thread-cross', 'plan', 'other-pln', 'other-pln', 'fp', 1, 1)",
                [],
            )
            .is_err()
        );
    }

    fn assert_stop_scope_index_exists(conn: &Connection) {
        let mut stmt = conn
            .prepare("PRAGMA index_list(stop_enforcement_state)")
            .unwrap();
        let names = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            names
                .iter()
                .any(|name| name == "idx_stop_enforcement_state_scope"),
            "missing stop scope index: {names:?}"
        );
    }

    fn assert_stop_table_sql_has_nonblank_session_check(conn: &Connection, table: &str) {
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("session_id TEXT NOT NULL CHECK(length(trim(session_id)) > 0)"),
            "{table} missing nonblank session CHECK: {sql}"
        );
    }

    fn assert_stop_integrity_triggers_exist(conn: &Connection) {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'stop_%_project_plan_%' ORDER BY name",
            )
            .unwrap();
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            names,
            vec![
                "stop_active_bindings_project_plan_insert",
                "stop_active_bindings_project_plan_update",
                "stop_enforcement_state_project_plan_insert",
                "stop_enforcement_state_project_plan_update",
            ],
            "missing stop integrity triggers"
        );
    }
}
