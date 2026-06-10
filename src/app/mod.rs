use crate::cli::Command;
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;

mod commands;
mod flow;
mod git_review;
mod graph;
mod http;
mod inspection;
mod lease;
mod mcp;
mod packages;
mod recall;
mod recovery;
mod render;
mod repository;
mod review;
mod review_workspace;
mod surfaces;

pub(crate) struct App {
    pub(crate) conn: Connection,
    pub(crate) root: PathBuf,
    pub(crate) db_path: PathBuf,
    pub(crate) json: bool,
}

fn artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "project_id": row.get::<_, String>(1)?,
        "item_id": row.get::<_, Option<String>>(2)?,
        "name": row.get::<_, String>(3)?,
        "kind": row.get::<_, Option<String>>(4)?,
        "path": row.get::<_, Option<String>>(5)?,
        "content": row.get::<_, Option<String>>(6)?,
        "mime_type": row.get::<_, Option<String>>(7)?,
        "size_bytes": row.get::<_, Option<i64>>(8)?,
        "metadata": parse_optional_json(row.get::<_, Option<String>>(9)?),
        "created_at": row.get::<_, String>(10)?,
    }))
}

fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "project_id": row.get::<_, Option<String>>(1)?,
        "item_id": row.get::<_, Option<String>>(2)?,
        "worker_id": row.get::<_, Option<String>>(3)?,
        "event_type": row.get::<_, String>(4)?,
        "payload": parse_optional_json(row.get::<_, Option<String>>(5)?),
        "timestamp": row.get::<_, String>(6)?,
    }))
}

fn parse_optional_json(raw: Option<String>) -> Value {
    raw.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

pub(crate) struct ReviewAnnotationInput<'a> {
    item_id: &'a str,
    message: &'a str,
    severity: &'a str,
    author: Option<&'a str>,
    file: Option<&'a str>,
    line: Option<u64>,
    source: &'a str,
}

impl App {
    pub(crate) fn dispatch(&self, command: Command) -> Result<()> {
        match command {
            Command::Project(args) => self.project(args.command),
            Command::Plan(args) => self.plan(args.command),
            Command::Map(args) => self.map(args.command),
            Command::Item(args) => self.item(args.command),
            Command::Link(args) => self.link(args.command),
            Command::Pick(args) => self.pick(args.command, args.work_type, args.plan),
            Command::Approval(args) => self.approval(args.command),
            Command::Log(args) => self.log(args.command),
            Command::Close(args) => self.close(args),
            Command::Done(args) => self.done(args),
            Command::Review(args) => self.review(args.command),
            Command::Context(args) => self.context(args.command),
            Command::Note(args) => self.context(args.command),
            Command::Search(args) => self.search(args),
            Command::Doctor(args) => self.doctor(args),
            Command::Install(args) => self.install(args.command),
            Command::Prompt(args) => self.prompt(args.command),
            Command::Mcp => self.mcp(),
            Command::Serve(args) => self.serve(args),
            Command::Trace(args) => self.trace(args.command),
            Command::Scrub(args) => self.scrub(args),
            Command::Artifact(args) => self.artifact(args.command),
            Command::Event(args) => self.event(args.command),
            Command::Debug(args) => self.debug(args.command),
            Command::Recover(args) => self.recover(args.command),
            Command::Export(args) => self.export(args),
            Command::Import(args) => self.import(args),
        }
    }
}
