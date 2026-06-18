use anyhow::Result;
use app::App;
use clap::Parser;
use cli::Cli;
use serde_json::json;
use std::{env, path::PathBuf};
use storage::{default_db_path, ensure_schema, open_db};
use util::{infer_error_code, print_json};

mod app;
mod cli;
mod integrations;
mod model;
mod planpack;
mod storage;
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
    let args = env::args().collect::<Vec<_>>();
    let root = env::current_dir()?;
    let db_path = parse_db_arg(&args).unwrap_or_else(|| default_db_path(&root));
    let conn = open_db(&db_path)?;
    ensure_schema(&conn)?;
    let app = App {
        conn,
        root,
        db_path,
        json: args.iter().any(|arg| arg == "--json"),
    };
    if app.try_raw_dispatch(&args[1..])? {
        return Ok(());
    }
    let cli = Cli::parse_from(args);
    app.dispatch(cli.command)
}

fn parse_db_arg(args: &[String]) -> Option<PathBuf> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(value) = arg.strip_prefix("--db=") {
            return Some(PathBuf::from(value));
        }
        if arg == "--db" {
            return args.get(i + 1).map(PathBuf::from);
        }
        i += 1;
    }
    None
}
