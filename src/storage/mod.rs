mod rows;
mod schema;

pub use rows::{row_to_context, row_to_item, row_to_log, row_to_plan, row_to_project};
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
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}
