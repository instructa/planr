use crate::model::{Item, Project};
use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use slug::slugify;
use std::{env, fs, path::Path};
use time::OffsetDateTime;
use uuid::Uuid;

pub fn collect_rows<T, E, I>(rows: I) -> Result<Vec<T>>
where
    I: IntoIterator<Item = std::result::Result<T, E>>,
    E: Into<anyhow::Error>,
{
    rows.into_iter()
        .map(|row| row.map_err(Into::into))
        .collect()
}

pub fn query_json(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    out: &mut Vec<Value>,
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok(json!({
            "type": row.get::<_, String>(0)?,
            "id": row.get::<_, String>(1)?,
            "title": row.get::<_, String>(2)?,
            "text": row.get::<_, String>(3)?,
        }))
    })?;
    for row in rows {
        out.push(row?);
    }
    Ok(())
}

pub fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn infer_error_code(message: &str) -> &'static str {
    if message.contains("not found") {
        "not_found"
    } else if message.contains("invalid_transition") {
        "invalid_transition"
    } else if message.contains("already_closed") {
        "already_closed"
    } else if message.contains("refusing")
        || message.contains("missing required")
        || message.contains("no project found")
    {
        "bad_request"
    } else if message.contains("locked") {
        "locked"
    } else if message.contains("parse") {
        "parse_error"
    } else {
        "internal_error"
    }
}

/// Percent-decode a URL query component, treating `+` as a space.
pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = [bytes[i + 1], bytes[i + 2]];
                match std::str::from_utf8(&hex)
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn format_project(project: &Project) -> String {
    format!("{} {} ({})", project.id, project.name, project.status)
}

pub fn format_item(item: &Item) -> String {
    format!("{} [{}] {}", item.id, item.status, item.title)
}

pub fn short_id(prefix: &str) -> String {
    let uuid = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &uuid[..8])
}

pub fn item_id(title: &str) -> String {
    let slug = slugify(title);
    let short = if slug.len() > 32 { &slug[..32] } else { &slug };
    // Truncation can land on a hyphen; trim it so ids never contain `--`.
    let short = short.trim_end_matches('-');
    format!("i-{short}-{}", &Uuid::new_v4().simple().to_string()[..4])
}

/// Title for a build plan split from a product plan. When the slice already
/// repeats the source title (e.g. slice "Habit Tracker MVP build"), the slice
/// is used as-is so plan slugs and filenames never duplicate the title.
pub fn split_plan_title(source_title: &str, slice: &str) -> String {
    if slugify(slice).starts_with(&slugify(source_title)) {
        slice.to_string()
    } else {
        format!("{source_title} - {slice}")
    }
}

pub fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown-time".to_string())
}

/// Worker identity must stay stable across tool processes within one agent
/// session, otherwise heartbeats and ownership checks fail mid-loop. Default
/// is client:host:user; agents should export PLANR_WORKER_ID (explicit
/// identity, e.g. `maker-1`) or PLANR_SESSION_ID for the whole session so
/// pick, done, and log all attribute to the same worker.
pub fn worker_id() -> String {
    if let Ok(id) = env::var("PLANR_WORKER_ID") {
        return id;
    }
    if let Ok(id) = env::var("PLANR_SESSION_ID") {
        return id;
    }
    if let Ok(id) = env::var("CODEX_SESSION_ID") {
        return format!("codex:{id}");
    }
    let host = env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string());
    let user = env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "worker".to_string());
    format!("{}:{}:{}", detect_client(), host, user)
}

pub fn detect_client() -> String {
    if env::var("CODEX_HOME").is_ok() {
        return "codex".to_string();
    }
    if env::var("CLAUDECODE").is_ok() {
        return "claude".to_string();
    }
    "human".to_string()
}

pub fn write_if_missing(path: &Path, content: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

pub fn append_line(path: &Path, content: &str) -> Result<()> {
    let mut existing = fs::read_to_string(path).unwrap_or_default();
    existing.push_str(content);
    fs::write(path, existing)?;
    Ok(())
}

pub fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| dir.join(name).exists()))
        .unwrap_or(false)
}

pub fn required_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required MCP argument: {name}"))
}

pub fn path_item_id(path: &str) -> Option<&str> {
    let parts = path.split('/').collect::<Vec<_>>();
    parts.windows(2).find_map(|window| {
        if window[0] == "items" {
            Some(window[1])
        } else {
            None
        }
    })
}

pub fn quote_fts(query: &str) -> String {
    let escaped = query.replace('"', "\"\"");
    format!("\"{escaped}\"")
}
