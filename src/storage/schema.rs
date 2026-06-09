use anyhow::Result;
use rusqlite::{params, Connection};

const SCHEMA_VERSION: i64 = 1;

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS projects(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  root_path TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL DEFAULT 'active',
  metadata TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS plans(
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
CREATE TABLE IF NOT EXISTS items(
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
CREATE TABLE IF NOT EXISTS links(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  from_item TEXT NOT NULL,
  to_item TEXT NOT NULL,
  kind TEXT NOT NULL,
  condition TEXT NOT NULL DEFAULT 'all',
  metadata TEXT,
  UNIQUE(from_item, to_item, kind)
);
CREATE TABLE IF NOT EXISTS source_links(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source_type TEXT NOT NULL,
  source_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  section_id TEXT,
  relationship TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS contexts(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  item_id TEXT,
  worker_id TEXT,
  kind TEXT NOT NULL,
  content TEXT NOT NULL,
  tags TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  client TEXT NOT NULL,
  profile TEXT,
  command TEXT,
  cwd TEXT,
  worktree_path TEXT,
  status TEXT NOT NULL,
  started_at TEXT,
  ended_at TEXT,
  exit_code INTEGER,
  metadata TEXT
);
CREATE TABLE IF NOT EXISTS logs(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  item_id TEXT NOT NULL,
  run_id TEXT,
  kind TEXT NOT NULL,
  summary TEXT NOT NULL,
  files TEXT,
  commands TEXT,
  tests TEXT,
  review_findings TEXT,
  blocked_or_unverified TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS artifacts(
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  item_id TEXT,
  name TEXT NOT NULL,
  kind TEXT,
  path TEXT,
  content TEXT,
  mime_type TEXT,
  size_bytes INTEGER,
  metadata TEXT,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id TEXT,
  item_id TEXT,
  worker_id TEXT,
  event_type TEXT NOT NULL,
  payload TEXT,
  timestamp TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(
  source_type UNINDEXED,
  source_id UNINDEXED,
  title,
  body,
  path UNINDEXED
);
"#,
    )?;
    ensure_column(conn, "items", "last_heartbeat_at", "TEXT")?;
    ensure_column(conn, "items", "progress_percent", "INTEGER")?;
    ensure_column(conn, "items", "progress_note", "TEXT")?;
    ensure_column(conn, "items", "paused_at", "TEXT")?;
    ensure_column(conn, "items", "timeout_seconds", "INTEGER")?;
    ensure_column(conn, "items", "max_retries", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(conn, "items", "retry_count", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(
        conn,
        "items",
        "retry_backoff",
        "TEXT NOT NULL DEFAULT 'exponential'",
    )?;
    ensure_column(
        conn,
        "items",
        "retry_delay_ms",
        "INTEGER NOT NULL DEFAULT 1000",
    )?;
    ensure_column(conn, "items", "pre_condition", "TEXT")?;
    ensure_column(conn, "items", "post_condition", "TEXT")?;
    ensure_column(conn, "items", "approval_status", "TEXT")?;
    ensure_column(conn, "items", "approval_requested_at", "TEXT")?;
    ensure_column(conn, "items", "approved_by", "TEXT")?;
    ensure_column(conn, "items", "approval_comment", "TEXT")?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
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
