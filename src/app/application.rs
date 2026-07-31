use super::{App, LogInput};
use crate::model::ApprovalStatus;
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow};
use rusqlite::params;
use serde_json::{Value, json};

pub(crate) struct ArtifactInput<'a> {
    pub(crate) item_id: Option<&'a str>,
    pub(crate) name: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) path: Option<&'a str>,
    pub(crate) content: Option<&'a str>,
    pub(crate) mime_type: &'a str,
    pub(crate) metadata: Value,
}

impl App {
    pub(crate) fn request_approval_value(
        &self,
        item_id: &str,
        reason: Option<&str>,
    ) -> Result<Value> {
        let changed = self.conn.execute(
            "UPDATE items SET approval_status = ?1, approval_requested_at = datetime('now'), approval_comment = ?2, approved_by = NULL, updated_at = datetime('now') WHERE id = ?3",
            params![ApprovalStatus::Requested.as_str(), reason, item_id],
        )?;
        ensure_item_changed(changed, item_id)?;
        Ok(json!({
            "item": self.get_item(item_id)?,
            "approval": self.item_approval(item_id)?,
            "proof": self.proof_status_for_item(item_id)?,
        }))
    }

    pub(crate) fn approve_value(
        &self,
        item_id: &str,
        by: &str,
        comment: Option<&str>,
    ) -> Result<Value> {
        self.set_approval(ApprovalStatus::Approved, item_id, by, comment)
    }

    pub(crate) fn deny_value(
        &self,
        item_id: &str,
        by: &str,
        comment: Option<&str>,
    ) -> Result<Value> {
        self.set_approval(ApprovalStatus::Denied, item_id, by, comment)
    }

    fn set_approval(
        &self,
        status: ApprovalStatus,
        item_id: &str,
        by: &str,
        comment: Option<&str>,
    ) -> Result<Value> {
        let changed = self.conn.execute(
            "UPDATE items SET approval_status = ?1, approved_by = ?2, approval_comment = ?3, updated_at = datetime('now') WHERE id = ?4",
            params![status.as_str(), by, comment, item_id],
        )?;
        ensure_item_changed(changed, item_id)?;
        Ok(json!({
            "item": self.get_item(item_id)?,
            "approval": self.item_approval(item_id)?,
            "proof": self.proof_status_for_item(item_id)?,
        }))
    }

    pub(crate) fn add_log_value(&self, input: LogInput<'_>) -> Result<Value> {
        let id = self.add_log_entry(input)?;
        self.get_log(&id)
    }

    pub(crate) fn close_item_value(&self, item_id: &str, source: &str) -> Result<Value> {
        let ready_before = self.ready_item_ids()?;
        self.close_item_core(item_id, source, false)?;
        Ok(json!({
            "closed": item_id,
            "unlocked": self.unlocked_since(&ready_before)?,
            "proof": self.proof_status_for_item(item_id)?,
        }))
    }

    pub(crate) fn add_context_value(
        &self,
        item_id: Option<&str>,
        kind: &str,
        content: &str,
        tags: Value,
        source: Option<&str>,
    ) -> Result<Value> {
        let id = short_id("ctx");
        self.conn.execute(
            "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
            params![id, self.default_project()?.id, item_id, worker_id(), kind, content, tags.to_string()],
        )?;
        self.index_search("context", &id, kind, content, None)?;
        let mut event = json!({"context_id": id.clone(), "tag": kind, "kind": kind});
        if let Some(source) = source {
            event["source"] = json!(source);
        }
        self.record_event("context_created", item_id, event)?;
        self.get_context(&id)
    }

    pub(crate) fn add_artifact_value(&self, input: ArtifactInput<'_>) -> Result<Value> {
        if let Some(item_id) = input.item_id {
            self.get_item(item_id)?;
        }
        let id = short_id("art");
        self.conn.execute(
            "INSERT INTO artifacts(id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
            params![
                id,
                self.default_project()?.id,
                input.item_id,
                input.name,
                input.kind,
                input.path,
                input.content,
                input.mime_type,
                input.content.map(|content| content.len() as i64),
                input.metadata.to_string(),
            ],
        )?;
        self.record_event(
            "artifact_created",
            input.item_id,
            json!({"artifact_id": id.clone(), "name": input.name}),
        )?;
        self.get_artifact(&id)
    }
}

fn ensure_item_changed(changed: usize, item_id: &str) -> Result<()> {
    if changed == 0 {
        Err(anyhow!("item not found: {item_id}"))
    } else {
        Ok(())
    }
}
