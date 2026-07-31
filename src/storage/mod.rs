mod eval_schema;
mod evidence_schema;
mod rows;
mod schema;
mod stop_schema;

#[allow(unused_imports)]
pub use rows::{row_to_context, row_to_item, row_to_log, row_to_plan, row_to_project};
#[allow(unused_imports)]
pub use schema::ensure_schema;

use anyhow::Result;
use rusqlite::Connection;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub fn default_db_path(root: &Path) -> PathBuf {
    if let Ok(path) = env::var("PLANR_DB") {
        return PathBuf::from(path);
    }
    root.join(".planr/planr.sqlite")
}

pub fn open_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    evidence_schema::register_connection_functions(&conn)?;
    // busy_timeout must be set before journal_mode: the WAL conversion
    // needs an exclusive lock, and with the default timeout of 0 two
    // processes opening a fresh database concurrently (parallel workers'
    // first pick) race into an immediate "database is locked" instead of
    // waiting out the moment the other one needs.
    conn.execute_batch(
        "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;",
    )?;
    Ok(conn)
}
