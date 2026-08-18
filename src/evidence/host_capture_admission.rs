use crate::evidence::model::Sha256Digest;
use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Debug)]
pub(crate) struct PendingHostCaptureAdmission {
    pub sealed_run_index_digest: String,
    pub project_id: String,
    pub plan_id: String,
    pub run_id: String,
    pub freeze_id: String,
    pub run_revision: i64,
    pub verifier_lease_generation: i64,
    pub obligation_id: String,
    pub execution_binding: Value,
    pub manifest: Value,
    pub instance: Value,
    pub normalized_capture_digest: String,
    pub valid_until: String,
    pub status: String,
}

impl PendingHostCaptureAdmission {
    pub(crate) fn validate_pending(&self) -> Result<()> {
        for digest in [&self.sealed_run_index_digest, &self.normalized_capture_digest] {
            Sha256Digest::parse(digest.clone()).map_err(|error| anyhow!(error))?;
        }
        if self.project_id.is_empty() || self.plan_id.is_empty() || self.run_id.is_empty()
            || self.freeze_id.is_empty() || self.obligation_id.is_empty() {
            bail!("host capture admission identity is required");
        }
        if self.run_revision < 0 || self.verifier_lease_generation < 0 || self.status != "pending" {
            bail!("host capture admission is not a valid pending record");
        }
        OffsetDateTime::parse(&self.valid_until, &Rfc3339)?;
        if !self.execution_binding.is_object() || !self.manifest.is_object() || !self.instance.is_object() {
            bail!("host capture admission snapshots must be JSON objects");
        }
        Ok(())
    }
}

pub(crate) fn insert_pending(conn: &Connection, value: &PendingHostCaptureAdmission) -> Result<()> {
    value.validate_pending()?;
    let admitted_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let changed = conn.execute(
        "INSERT INTO host_capture_admissions(sealed_run_index_digest,project_id,plan_id,run_id,freeze_id,run_revision,verifier_lease_generation,obligation_id,execution_binding_json,manifest_json,instance_json,normalized_capture_digest,valid_until,status,admitted_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'pending',?14)",
        params![value.sealed_run_index_digest,value.project_id,value.plan_id,value.run_id,value.freeze_id,value.run_revision,value.verifier_lease_generation,value.obligation_id,serde_json::to_string(&value.execution_binding)?,serde_json::to_string(&value.manifest)?,serde_json::to_string(&value.instance)?,value.normalized_capture_digest,value.valid_until,admitted_at],
    )?;
    if changed != 1 { bail!("host capture admission insert did not create one record"); }
    Ok(())
}

pub(crate) fn load_pending(conn: &Connection, digest: &str) -> Result<Option<PendingHostCaptureAdmission>> {
    conn.query_row("SELECT sealed_run_index_digest,project_id,plan_id,run_id,freeze_id,run_revision,verifier_lease_generation,obligation_id,execution_binding_json,manifest_json,instance_json,normalized_capture_digest,valid_until,status FROM host_capture_admissions WHERE sealed_run_index_digest=?1 AND status='pending'", params![digest], |row| Ok(PendingHostCaptureAdmission { sealed_run_index_digest: row.get(0)?, project_id: row.get(1)?, plan_id: row.get(2)?, run_id: row.get(3)?, freeze_id: row.get(4)?, run_revision: row.get(5)?, verifier_lease_generation: row.get(6)?, obligation_id: row.get(7)?, execution_binding: serde_json::from_str(&row.get::<_,String>(8)?).map_err(|e| rusqlite::Error::FromSqlConversionFailure(8,rusqlite::types::Type::Text,Box::new(e)))?, manifest: serde_json::from_str(&row.get::<_,String>(9)?).map_err(|e| rusqlite::Error::FromSqlConversionFailure(9,rusqlite::types::Type::Text,Box::new(e)))?, instance: serde_json::from_str(&row.get::<_,String>(10)?).map_err(|e| rusqlite::Error::FromSqlConversionFailure(10,rusqlite::types::Type::Text,Box::new(e)))?, normalized_capture_digest: row.get(11)?, valid_until: row.get(12)?, status: row.get(13)? })).optional().map_err(Into::into)
}

pub(crate) fn mark_promoted(conn: &Connection, digest: &str) -> Result<()> {
    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
    if conn.execute("UPDATE host_capture_admissions SET status='promoted', promoted_at=?2 WHERE sealed_run_index_digest=?1 AND status='pending'", params![digest, now])? != 1 { bail!("host capture admission is not pending"); }
    Ok(())
}
