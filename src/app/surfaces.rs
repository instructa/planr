use super::App;
use crate::cli::{ArtifactCommand, DebugCommand, EventCommand, ScrubArgs, TraceCommand};
use crate::util::short_id;
use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::json;
use std::fs;

impl App {
    pub(crate) fn trace(&self, command: TraceCommand) -> Result<()> {
        match command {
            TraceCommand::Item(args) => {
                let item = self.get_item(&args.id)?;
                let logs = self.list_logs(Some(&args.id))?;
                let links = self.links_for(&args.id)?;
                self.emit(
                    json!({
                        "item": item,
                        "links": links,
                        "logs": logs,
                        "runtime": self.item_runtime(&args.id)?,
                        "recovery": self.item_recovery(&args.id)?,
                        "conditions": self.item_conditions(&args.id)?,
                        "approval": self.item_approval(&args.id)?,
                    }),
                    "trace complete".to_string(),
                )
            }
        }
    }

    pub(crate) fn scrub(&self, args: ScrubArgs) -> Result<()> {
        let findings = self.secret_findings()?;
        if args.preview || !args.confirm {
            return self.emit(
                json!({"findings": findings, "mode": "preview"}),
                format!("{} possible secret(s)", findings.len()),
            );
        }
        let scrubbed = self.apply_scrub()?;
        self.emit(
            json!({"findings": findings, "mode": "confirm", "scrubbed": scrubbed}),
            format!("scrub complete: {scrubbed} value(s) redacted"),
        )
    }

    pub(crate) fn artifact(&self, command: ArtifactCommand) -> Result<()> {
        match command {
            ArtifactCommand::Add(args) => {
                let name = args.name.clone().or(args.name_flag.clone()).ok_or_else(|| {
                    anyhow!(
                        "artifact name is required: `planr artifact add \"<name>\" ...` or `planr artifact add --name \"<name>\" ...`"
                    )
                })?;
                if args.path.is_some() && args.content.is_some() {
                    bail!("provide --path or --content, not both");
                }
                if let Some(item_id) = args.item.as_deref() {
                    self.get_item(item_id)?;
                }
                let id = short_id("art");
                let path_string = args
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string());
                let size_bytes = if let Some(path) = args.path.as_ref() {
                    fs::metadata(path)
                        .map(|metadata| metadata.len() as i64)
                        .ok()
                } else {
                    args.content.as_ref().map(|content| content.len() as i64)
                };
                self.conn.execute(
                    "INSERT INTO artifacts(id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
                    params![
                        id,
                        self.default_project()?.id,
                        args.item.as_deref(),
                        name,
                        args.kind.as_deref().unwrap_or("evidence"),
                        path_string,
                        args.content,
                        args.mime.as_deref().unwrap_or("text/plain"),
                        size_bytes,
                        json!({"storage": if args.path.is_some() { "path" } else { "inline" }}).to_string(),
                    ],
                )?;
                self.record_event(
                    "artifact_created",
                    args.item.as_deref(),
                    json!({"artifact_id": id, "name": name, "kind": args.kind}),
                )?;
                self.emit(
                    json!({"artifact": self.get_artifact(&id)?}),
                    format!("artifact {id} added"),
                )
            }
            ArtifactCommand::Show(args) => {
                let artifact = self.get_artifact(&args.id)?;
                self.emit(
                    json!({"artifact": artifact}),
                    format!("artifact {}", args.id),
                )
            }
            ArtifactCommand::List(args) => {
                let artifacts = self.list_artifacts(args.item.as_deref())?;
                self.emit(
                    json!({"artifacts": artifacts}),
                    format!("{} artifact(s)", artifacts.len()),
                )
            }
        }
    }

    pub(crate) fn event(&self, command: EventCommand) -> Result<()> {
        match command {
            EventCommand::List(args) => {
                let events = self.list_events(args.item.as_deref(), args.limit)?;
                self.emit(
                    json!({"events": events}),
                    format!("{} event(s)", events.len()),
                )
            }
        }
    }

    pub(crate) fn debug(&self, command: DebugCommand) -> Result<()> {
        match command {
            DebugCommand::Bundle(args) => {
                if !args.preview {
                    bail!("debug bundle writes are not enabled in V1; use --preview");
                }
                let bundle = self.debug_bundle(args.item.as_deref())?;
                self.emit(bundle, "debug bundle preview".to_string())
            }
        }
    }
}
