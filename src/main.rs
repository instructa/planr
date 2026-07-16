use anyhow::Result;
use app::App;
use clap::Parser;
use cli::Cli;
use serde_json::json;
use std::env;
use storage::{default_db_path, ensure_schema, open_db};
use util::{infer_error_code, print_json};

mod agents;
mod app;
mod cli;
pub mod execution_policy;
mod integrations;
mod model;
mod planpack;
mod preset;
mod preset_catalog;
mod preset_eval;
mod preset_registry;
mod rolefiles;
mod route_audit;
mod secrets;
mod storage;
pub mod usage_policy;
mod util;

fn main() {
    if let Err(err) = run() {
        let json_mode = env::args().any(|arg| arg == "--json");
        if json_mode {
            let message = err.to_string();
            let _ = print_json(
                &json!({"error": {"code": infer_error_code(&message), "message": message}}),
            );
        } else {
            eprintln!("error: {err:#}");
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = env::current_dir()?;
    let db_path = cli.db.clone().unwrap_or_else(|| default_db_path(&root));
    // Prime is wired into host hooks that fire in every repo, planr
    // project or not: without a database it must exit silently instead
    // of creating one (open_db creates the file).
    if matches!(cli.command, cli::Command::Prime(_)) && !db_path.exists() {
        return Ok(());
    }
    let conn = open_db(&db_path)?;
    ensure_schema(&conn)?;
    let app = App::new(conn, root, db_path, cli.json);
    app.dispatch(cli.command)
}
