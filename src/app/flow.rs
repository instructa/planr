use super::App;
use super::lease::PickFilter;
use crate::cli::{DoneArgs, DoneEscalationReason};
use crate::model::ItemStatus;
use crate::route_audit::{RouteObservation, load_route_observation};
use crate::usage_policy::{
    ChangeKind, ChangeSummary, MaterialityPolicy, MaterialityTrigger, PolicyLoad,
    ReviewRequirement, RiskLevel, classify_materiality, load_policy,
};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

/// The profile a run actually executed on: explicit input wins, then the
/// `PLANR_PROFILE` environment variable (role files rendered from the
/// registry can export it), else none — and none means no comparison and
/// no event, never a guess.
fn effective_profile(explicit: Option<&str>) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            std::env::var("PLANR_PROFILE")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

fn review_requirement_str(requirement: ReviewRequirement) -> &'static str {
    match requirement {
        ReviewRequirement::None => "none",
        ReviewRequirement::IndependentHighSignal => "independent_high_signal",
    }
}

fn change_kind_str(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Code => "code",
        ChangeKind::Documentation => "documentation",
        ChangeKind::Formatting => "formatting",
        ChangeKind::TestsOnly => "tests_only",
    }
}

fn materiality_trigger_str(trigger: MaterialityTrigger) -> &'static str {
    trigger.as_str()
}

fn unique_files(files: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    files
        .iter()
        .map(|file| file.trim())
        .filter(|file| !file.is_empty())
        .filter(|file| seen.insert((*file).to_string()))
        .map(ToOwned::to_owned)
        .collect()
}

fn json_string_array(raw: Option<String>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
}

fn classify_change_kind(files: &[String]) -> ChangeKind {
    if files.is_empty() {
        return ChangeKind::Code;
    }
    let all_docs = files.iter().all(|file| {
        file.starts_with("docs/")
            || file.starts_with("apps/docs/content/")
            || file.ends_with(".md")
            || file.ends_with(".mdx")
            || file.ends_with(".txt")
    });
    if all_docs {
        return ChangeKind::Documentation;
    }
    let all_tests = files.iter().all(|file| {
        file.starts_with("tests/")
            || file.starts_with("scripts/test-")
            || file.contains("_test.")
            || file.contains(".test.")
    });
    if all_tests {
        return ChangeKind::TestsOnly;
    }
    ChangeKind::Code
}

fn infer_materiality_triggers(files: &[String]) -> BTreeSet<MaterialityTrigger> {
    let mut triggers = BTreeSet::new();
    for file in files {
        let lower = file.to_ascii_lowercase();
        if lower.contains("auth")
            || lower.contains("secret")
            || lower.contains("crypto")
            || lower.contains("token")
            || lower.contains("password")
        {
            triggers.insert(MaterialityTrigger::SecurityOrAuth);
        }
        if lower.contains("schema")
            || lower.contains("migration")
            || lower.starts_with("src/storage/")
            || lower.contains("database")
        {
            triggers.insert(MaterialityTrigger::SchemaOrMigration);
        }
        if lower.starts_with(".github/")
            || lower.starts_with("scripts/release")
            || lower.starts_with("scripts/build-release")
            || lower.contains("deploy")
            || lower.contains("docker")
        {
            triggers.insert(MaterialityTrigger::InfrastructureOrDeploy);
        }
        if lower.starts_with("src/cli")
            || lower.starts_with("src/app/http")
            || lower.starts_with("src/app/mcp")
            || lower.starts_with("src/integrations/mcp")
            || lower.starts_with("apps/docs/content/docs/reference/")
        {
            triggers.insert(MaterialityTrigger::PublicApi);
        }
        if lower.contains("billing") {
            triggers.insert(MaterialityTrigger::Billing);
        }
        if lower.starts_with("src/app/flow")
            || lower.starts_with("src/app/review")
            || lower.starts_with("src/app/repository")
            || lower.starts_with("src/app/lease")
        {
            triggers.insert(MaterialityTrigger::ConcurrencyOrTransaction);
        }
        if lower == "cargo.lock" || lower == "pnpm-lock.yaml" || lower.ends_with("package.json") {
            triggers.insert(MaterialityTrigger::LargeDependencyChange);
        }
    }
    triggers
}

struct LineEvidence {
    changed_lines: u32,
    reasons: Vec<String>,
}

enum ClaimState {
    Untracked,
    Unchanged,
    Missing,
}

fn git_changed_lines(root: &Path, files: &[String]) -> LineEvidence {
    if files.is_empty() {
        return LineEvidence {
            changed_lines: 0,
            reasons: Vec::new(),
        };
    }
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .arg("diff")
        .arg("--numstat")
        .arg("HEAD")
        .arg("--");
    for file in files {
        command.arg(file);
    }
    let output = match command.output() {
        Ok(output) => output,
        Err(_) => {
            return LineEvidence {
                changed_lines: 0,
                reasons: vec!["changed_lines_unknown:git_unavailable".to_string()],
            };
        }
    };
    if !output.status.success() {
        return LineEvidence {
            changed_lines: 0,
            reasons: vec!["changed_lines_unknown:git_error".to_string()],
        };
    }
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(_) => {
            return LineEvidence {
                changed_lines: 0,
                reasons: vec!["changed_lines_unknown:git_output".to_string()],
            };
        }
    };
    let mut total = 0u32;
    let mut observed = BTreeSet::new();
    for line in stdout.lines() {
        let mut columns = line.split('\t');
        let additions = columns.next().unwrap_or("0");
        let deletions = columns.next().unwrap_or("0");
        if let Some(file) = columns.next() {
            observed.insert(file.to_string());
        }
        for value in [additions, deletions] {
            if value != "-" {
                total = total.saturating_add(value.parse::<u32>().unwrap_or(0));
            }
        }
    }
    let mut reasons = Vec::new();
    for file in files {
        if !observed.contains(file) {
            match git_claim_state(root, file) {
                ClaimState::Untracked => {
                    total = total.saturating_add(count_file_lines(root, file));
                }
                ClaimState::Unchanged => reasons.push(format!("unchanged_claimed_file:{file}")),
                ClaimState::Missing => reasons.push(format!("missing_claimed_file:{file}")),
            }
        }
    }
    LineEvidence {
        changed_lines: total,
        reasons,
    }
}

fn git_claim_state(root: &Path, file: &str) -> ClaimState {
    if let Ok(output) = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain", "--", file])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.lines().any(|line| line.starts_with("??")) {
                return ClaimState::Untracked;
            }
        }
    }
    if root.join(file).exists() {
        ClaimState::Unchanged
    } else {
        ClaimState::Missing
    }
}

fn count_file_lines(root: &Path, file: &str) -> u32 {
    fs::read_to_string(root.join(file))
        .map(|contents| {
            if contents.is_empty() {
                0
            } else {
                contents
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    .saturating_add(usize::from(!contents.ends_with('\n')))
            }
        })
        .unwrap_or(0)
        .try_into()
        .unwrap_or(u32::MAX)
}

fn derive_change_summary(
    root: &Path,
    files: &[String],
    summary: &str,
    commands: &[String],
    tests: &[String],
    policy_loaded: bool,
) -> (ChangeSummary, Vec<String>) {
    let files = unique_files(files);
    let mut reasons = Vec::new();
    if policy_loaded
        && files.is_empty()
        && (!summary.trim().is_empty() || !commands.is_empty() || !tests.is_empty())
    {
        reasons.push("missing_changed_files".to_string());
    }
    let line_evidence = git_changed_lines(root, &files);
    reasons.extend(line_evidence.reasons);
    let kind = classify_change_kind(&files);
    let triggers = infer_materiality_triggers(&files);
    let risk = if !reasons.is_empty() || !triggers.is_empty() {
        RiskLevel::High
    } else if matches!(
        kind,
        ChangeKind::Documentation | ChangeKind::Formatting | ChangeKind::TestsOnly
    ) {
        RiskLevel::Low
    } else {
        RiskLevel::Moderate
    };
    (
        ChangeSummary {
            risk,
            triggers,
            changed_files: files.len() as u32,
            changed_lines: line_evidence.changed_lines,
            kind,
        },
        reasons,
    )
}

struct MaterialityValueInput<'a> {
    root: &'a Path,
    policy: &'a MaterialityPolicy,
    policy_id: &'a str,
    policy_version: &'a str,
    item_id: &'a str,
    summary: &'a str,
    files: &'a [String],
    commands: &'a [String],
    tests: &'a [String],
    explicit_review: bool,
}

struct UnavailablePolicyMaterialityInput<'a> {
    item_id: &'a str,
    summary: &'a str,
    files: &'a [String],
    commands: &'a [String],
    tests: &'a [String],
    explicit_review: bool,
    reason: &'a str,
    diagnostics: Option<String>,
}

struct RecordedCompletionFacts {
    summary: String,
    files: Vec<String>,
    commands: Vec<String>,
    tests: Vec<String>,
}

fn materiality_value(input: MaterialityValueInput<'_>) -> Value {
    let (change, mut derivation_reasons) = derive_change_summary(
        input.root,
        input.files,
        input.summary,
        input.commands,
        input.tests,
        true,
    );
    let mut decision = classify_materiality(input.policy, &change);
    decision.reasons.append(&mut derivation_reasons);
    let computed_requires_review = decision.review == ReviewRequirement::IndependentHighSignal;
    let required = computed_requires_review || input.explicit_review;
    let effective_reason = match (computed_requires_review, input.explicit_review) {
        (true, true) => "computed_materiality_and_explicit_escalation",
        (true, false) => "computed_materiality",
        (false, true) => "explicit_escalation",
        (false, false) => "computed_non_material",
    };
    json!({
        "schema_version": 1,
        "source": "settlement",
        "item_id": input.item_id,
        "policy": {
            "id": input.policy_id,
            "version": input.policy_version,
            "enforcement": "available",
        },
        "change_summary": {
            "risk": change.risk.as_str(),
            "triggers": change.triggers.iter().copied().map(materiality_trigger_str).collect::<Vec<_>>(),
            "changed_files": change.changed_files,
            "changed_lines": change.changed_lines,
            "kind": change_kind_str(change.kind),
            "files": unique_files(input.files),
        },
        "decision": {
            "material": decision.material,
            "review": review_requirement_str(decision.review),
            "reasons": decision.reasons,
        },
        "effective_review": {
            "required": required,
            "reason": effective_reason,
            "explicit_escalation": input.explicit_review,
        }
    })
}

pub(crate) struct LogInput<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) files: &'a [String],
    pub(crate) commands: &'a [String],
    pub(crate) tests: &'a [String],
    pub(crate) source: Option<&'a str>,
    /// Registry profile the work actually executed on, when the worker
    /// knows it (`--profile` flag; `PLANR_PROFILE` env is the fallback).
    pub(crate) profile: Option<&'a str>,
    pub(crate) route_observation: Option<&'a RouteObservation>,
}

pub(crate) struct SurfaceCompletionInput<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) files: &'a [String],
    pub(crate) commands: &'a [String],
    pub(crate) tests: &'a [String],
    pub(crate) source: &'a str,
    pub(crate) profile: Option<&'a str>,
    pub(crate) escalation_reason: Option<&'a str>,
    pub(crate) escalation_reference: Option<&'a str>,
    pub(crate) escalation_explanation: Option<&'a str>,
    pub(crate) write_log: bool,
}

/// Owner of the compound work flow: evidence logging, canonical run
/// settlement, and the `done` command that chains them. CLI, HTTP, and
/// MCP surfaces call these instead of duplicating the steps.
impl App {
    fn surface_escalation(
        reason: Option<&str>,
        reference: Option<&str>,
        explanation: Option<&str>,
    ) -> Result<Option<crate::usage_policy::ReviewEscalation>> {
        let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) else {
            if reference.is_some() || explanation.is_some() {
                bail!("structured_escalation_reason_required");
            }
            return Ok(None);
        };
        let reason = match reason {
            "user_requested" => crate::usage_policy::ReviewEscalationReason::UserRequested,
            "policy_required" => crate::usage_policy::ReviewEscalationReason::PolicyRequired,
            "protected_risk_discovered" => {
                crate::usage_policy::ReviewEscalationReason::ProtectedRiskDiscovered
            }
            "external_side_effect" => {
                crate::usage_policy::ReviewEscalationReason::ExternalSideEffect
            }
            "data_integrity_risk" => crate::usage_policy::ReviewEscalationReason::DataIntegrityRisk,
            value => bail!("invalid_escalation_reason:{value}"),
        };
        let source = match reason {
            crate::usage_policy::ReviewEscalationReason::PolicyRequired => {
                crate::usage_policy::EscalationSource::Policy
            }
            crate::usage_policy::ReviewEscalationReason::ProtectedRiskDiscovered
            | crate::usage_policy::ReviewEscalationReason::DataIntegrityRisk => {
                crate::usage_policy::EscalationSource::MakerFinding
            }
            crate::usage_policy::ReviewEscalationReason::UserRequested
            | crate::usage_policy::ReviewEscalationReason::ExternalSideEffect => {
                crate::usage_policy::EscalationSource::User
            }
        };
        Ok(Some(crate::usage_policy::ReviewEscalation {
            reason,
            source,
            reference: reference.unwrap_or_default().to_string(),
            explanation: explanation.unwrap_or_default().to_string(),
        }))
    }

    fn done_escalation(args: &DoneArgs) -> Result<Option<crate::usage_policy::ReviewEscalation>> {
        let Some(reason) = args.escalate else {
            if args.escalation_ref.is_some() || args.escalation_explanation.is_some() {
                bail!("structured_escalation_reason_required");
            }
            return Ok(None);
        };
        let reason = match reason {
            DoneEscalationReason::UserRequested => {
                crate::usage_policy::ReviewEscalationReason::UserRequested
            }
            DoneEscalationReason::PolicyRequired => {
                crate::usage_policy::ReviewEscalationReason::PolicyRequired
            }
            DoneEscalationReason::ProtectedRiskDiscovered => {
                crate::usage_policy::ReviewEscalationReason::ProtectedRiskDiscovered
            }
            DoneEscalationReason::ExternalSideEffect => {
                crate::usage_policy::ReviewEscalationReason::ExternalSideEffect
            }
            DoneEscalationReason::DataIntegrityRisk => {
                crate::usage_policy::ReviewEscalationReason::DataIntegrityRisk
            }
        };
        let source = match reason {
            crate::usage_policy::ReviewEscalationReason::PolicyRequired => {
                crate::usage_policy::EscalationSource::Policy
            }
            crate::usage_policy::ReviewEscalationReason::ProtectedRiskDiscovered
            | crate::usage_policy::ReviewEscalationReason::DataIntegrityRisk => {
                crate::usage_policy::EscalationSource::MakerFinding
            }
            crate::usage_policy::ReviewEscalationReason::UserRequested
            | crate::usage_policy::ReviewEscalationReason::ExternalSideEffect => {
                crate::usage_policy::EscalationSource::User
            }
        };
        Ok(Some(crate::usage_policy::ReviewEscalation {
            reason,
            source,
            reference: args.escalation_ref.clone().unwrap_or_default(),
            explanation: args.escalation_explanation.clone().unwrap_or_default(),
        }))
    }

    pub(crate) fn settle_surface_completion_value(
        &self,
        input: SurfaceCompletionInput<'_>,
    ) -> Result<Value> {
        let escalation = Self::surface_escalation(
            input.escalation_reason,
            input.escalation_reference,
            input.escalation_explanation,
        )?;
        self.ensure_outcome_feature_run(input.item_id)?;
        let ready_before = self.ready_item_ids()?;
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT surface_settlement")?;
        let result = (|| -> Result<Value> {
            self.adopt_ready_item(input.item_id)?;
            let materiality = self.settlement_materiality(
                input.item_id,
                input.summary,
                input.files,
                input.commands,
                input.tests,
                escalation.is_some(),
            );
            let log_id = if input.write_log {
                Some(self.add_log_entry(LogInput {
                    item_id: input.item_id,
                    kind: "completion",
                    summary: input.summary,
                    files: input.files,
                    commands: input.commands,
                    tests: input.tests,
                    source: Some(input.source),
                    profile: input.profile,
                    route_observation: None,
                })?)
            } else {
                None
            };
            let work_packet =
                self.settle_feature_run_outcome(super::execution_run::OutcomeSettlement {
                    item_id: input.item_id,
                    summary: input.summary,
                    materiality: &materiality,
                    escalation,
                })?;
            self.close_item_core(input.item_id, input.summary, false)?;
            self.persist_settlement_materiality(input.item_id, &materiality)?;
            Ok(json!({
                "closed": input.item_id,
                "item": self.get_item(input.item_id)?,
                "log_id": log_id,
                "work_packet": work_packet,
                "materiality": materiality,
                "unlocked": self.unlocked_since(&ready_before)?,
                "proof": self.proof_status_for_item(input.item_id)?,
                "remaining": self.progress_value()?,
            }))
        })();
        match result {
            Ok(value) => {
                self.conn
                    .execute_batch("RELEASE surface_settlement; COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO surface_settlement; RELEASE surface_settlement; ROLLBACK",
                );
                Err(error)
            }
        }
    }

    pub(crate) fn close_item_with_materiality_value(
        &self,
        item_id: &str,
        source: &str,
    ) -> Result<Value> {
        let facts = self.recorded_completion_facts(item_id)?;
        let summary = if facts.summary.trim().is_empty() {
            source
        } else {
            facts.summary.as_str()
        };
        self.settle_surface_completion_value(SurfaceCompletionInput {
            item_id,
            summary,
            files: &facts.files,
            commands: &facts.commands,
            tests: &facts.tests,
            source,
            profile: None,
            escalation_reason: None,
            escalation_reference: None,
            escalation_explanation: None,
            write_log: false,
        })
    }

    fn settlement_materiality(
        &self,
        item_id: &str,
        summary: &str,
        files: &[String],
        commands: &[String],
        tests: &[String],
        explicit_review: bool,
    ) -> Value {
        match load_policy(&self.root) {
            PolicyLoad::Loaded(policy) => materiality_value(MaterialityValueInput {
                root: &self.root,
                policy: &policy.materiality,
                policy_id: &policy.id,
                policy_version: &policy.version,
                item_id,
                summary,
                files,
                commands,
                tests,
                explicit_review,
            }),
            PolicyLoad::Missing => {
                self.unavailable_policy_materiality(UnavailablePolicyMaterialityInput {
                    item_id,
                    summary,
                    files,
                    commands,
                    tests,
                    explicit_review,
                    reason: "missing",
                    diagnostics: None,
                })
            }
            PolicyLoad::Invalid(diagnostics) => {
                self.unavailable_policy_materiality(UnavailablePolicyMaterialityInput {
                    item_id,
                    summary,
                    files,
                    commands,
                    tests,
                    explicit_review,
                    reason: "invalid",
                    diagnostics: Some(diagnostics.to_string()),
                })
            }
        }
    }

    fn unavailable_policy_materiality(
        &self,
        input: UnavailablePolicyMaterialityInput<'_>,
    ) -> Value {
        let (change, mut reasons) = derive_change_summary(
            &self.root,
            input.files,
            input.summary,
            input.commands,
            input.tests,
            false,
        );
        reasons.push(format!("policy_unavailable:{}", input.reason));
        let effective_reason = match (input.reason, input.explicit_review) {
            (_, true) => "structured_escalation",
            ("missing", false) => "policy_missing_operational_gap",
            ("invalid", false) => "policy_invalid_operational_gap",
            _ => "policy_unavailable_operational_gap",
        };
        let mut policy = json!({
            "id": Value::Null,
            "version": Value::Null,
            "enforcement": "unavailable",
            "reason": input.reason,
        });
        if let Some(diagnostics) = input.diagnostics {
            policy["diagnostics"] = json!(diagnostics);
        }
        json!({
            "schema_version": 1,
            "source": "settlement",
            "item_id": input.item_id,
            "policy": policy,
            "change_summary": {
                "risk": "high",
                "triggers": change.triggers.iter().copied().map(materiality_trigger_str).collect::<Vec<_>>(),
                "changed_files": change.changed_files,
                "changed_lines": change.changed_lines,
                "kind": change_kind_str(change.kind),
                "files": unique_files(input.files),
            },
            "decision": {
                "material": false,
                "review": "none",
                "reasons": reasons,
            },
            "effective_review": {
                "required": input.explicit_review,
                "reason": effective_reason,
                "explicit_escalation": input.explicit_review,
            }
        })
    }

    fn persist_settlement_materiality(&self, item_id: &str, value: &Value) -> Result<()> {
        self.set_item_metadata_field(item_id, "materiality", value.clone())?;
        self.record_event("materiality_decided", Some(item_id), value.clone())?;
        Ok(())
    }

    fn latest_completion_log_id(&self, item_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM logs WHERE item_id = ?1 AND kind = 'completion' ORDER BY created_at DESC, id DESC LIMIT 1",
                params![item_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    fn recorded_completion_facts(&self, item_id: &str) -> Result<RecordedCompletionFacts> {
        let mut stmt = self.conn.prepare(
            "SELECT summary, files, commands, tests FROM logs WHERE item_id = ?1 AND kind = 'completion' ORDER BY created_at, id",
        )?;
        let mut rows = stmt.query(params![item_id])?;
        let mut summaries = Vec::new();
        let mut files = Vec::new();
        let mut commands = Vec::new();
        let mut tests = Vec::new();
        while let Some(row) = rows.next()? {
            let summary: String = row.get(0)?;
            if !summary.trim().is_empty() {
                summaries.push(summary);
            }
            files.extend(json_string_array(row.get::<_, Option<String>>(1)?));
            commands.extend(json_string_array(row.get::<_, Option<String>>(2)?));
            tests.extend(json_string_array(row.get::<_, Option<String>>(3)?));
        }
        Ok(RecordedCompletionFacts {
            summary: summaries.join("; "),
            files,
            commands,
            tests,
        })
    }

    /// Single owner for writing evidence logs. Logging from the pick owner
    /// also refreshes the runtime heartbeat: evidence is a liveness signal,
    /// so agents do not need a separate `pick heartbeat` call.
    pub(crate) fn add_log_entry(&self, input: LogInput<'_>) -> Result<String> {
        if input.route_observation.is_some() && input.commands.is_empty() && input.tests.is_empty()
        {
            bail!("route audit requires a recorded run with at least one command or test");
        }
        let id = short_id("log");
        let profile = effective_profile(input.profile);
        let run_id = if input.commands.is_empty() && input.tests.is_empty() {
            None
        } else {
            Some(self.record_run(
                input.item_id,
                input.commands,
                "closed",
                profile.as_deref(),
                input.route_observation,
            )?)
        };
        if let Some(run_id) = run_id.as_deref() {
            if let Some(actual_profile) = profile.as_deref() {
                self.observe_route_compliance(input.item_id, run_id, actual_profile, input.kind)?;
            }
            // Client compliance needs no profile: the observed host comes
            // from the environment, so it also audits runs whose worker
            // never reported a profile.
            self.observe_client_compliance(input.item_id, run_id, input.kind)?;
            if let Some(observation) = input.route_observation {
                self.record_route_observation_events(input.item_id, run_id, observation)?;
            }
        }
        self.conn.execute(
            "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, files, commands, tests, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            params![
                id,
                self.default_project()?.id,
                input.item_id,
                run_id,
                input.kind,
                input.summary,
                serde_json::to_string(
                    &input.files
                        .iter()
                        .map(|file| file.trim())
                        .filter(|file| !file.is_empty())
                        .collect::<Vec<_>>(),
                )?,
                serde_json::to_string(input.commands)?,
                serde_json::to_string(input.tests)?,
            ],
        )?;
        self.index_search("log", &id, input.summary, input.summary, None)?;
        let mut event = json!({"log_id": id, "kind": input.kind});
        if let Some(source) = input.source {
            event["source"] = json!(source);
        }
        self.record_event("log_created", Some(input.item_id), event)?;
        self.conn.execute(
            "UPDATE items SET status = CASE WHEN status = 'picked' THEN 'running' ELSE status END, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND worker_id = ?2 AND status IN ('picked','running','in_review')",
            params![input.item_id, worker_id()],
        )?;
        Ok(id)
    }

    /// Single owner for the close transition. Reconciles gate state first so
    /// a parent whose children are already settled is closable instead of
    /// stuck in `blocked`, then validates, closes, and promotes unlocked work.
    pub(crate) fn close_item_core(
        &self,
        item_id: &str,
        summary: &str,
        write_log: bool,
    ) -> Result<Option<String>> {
        self.promote_ready()?;
        self.ensure_can_close(item_id)?;
        self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item_id])?;
        let log_id = if write_log {
            Some(self.add_log_entry(LogInput {
                item_id,
                kind: "completion",
                summary,
                files: &[],
                commands: &[],
                tests: &[],
                source: None,
                profile: None,
                route_observation: None,
            })?)
        } else {
            None
        };
        self.promote_ready()?;
        self.record_event(
            "item_closed",
            Some(item_id),
            json!({"log_id": log_id, "summary": summary}),
        )?;
        Ok(log_id)
    }

    /// Completion-time context shared by `done` and `close`: what the
    /// settlement unlocked, the post condition to verify, and an evidence
    /// hint when downstream work exists but no commands/tests were logged.
    pub(crate) fn settlement_extras(
        &self,
        item_id: &str,
        ready_before: &std::collections::HashSet<String>,
        has_evidence: bool,
    ) -> Result<Value> {
        let unlocked = self.unlocked_since(ready_before)?;
        let post_condition = self.item_conditions(item_id)?["post"]
            .as_str()
            .map(ToOwned::to_owned);
        let downstream: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM links WHERE from_item = ?1 AND kind = 'blocks'",
            params![item_id],
            |row| row.get(0),
        )?;
        let hint = (!has_evidence && downstream > 0).then(|| {
            format!(
                "downstream items depend on {item_id} but this log has no --cmd/--tests evidence; attach it with `planr log add {item_id} --summary ... --cmd ...`"
            )
        });
        Ok(json!({
            "unlocked": unlocked,
            "post_condition": post_condition,
            "hint": hint,
            "proof": self.proof_status_for_item(item_id)?,
        }))
    }

    pub(crate) fn done(&self, args: DoneArgs) -> Result<()> {
        let escalation = Self::done_escalation(&args)?;
        let route_observation = args
            .route_audit
            .as_deref()
            .map(load_route_observation)
            .transpose()?;
        let item_id = if let Some(id) = args.item_id {
            id
        } else {
            self.current_item_for_worker()?
                .ok_or_else(|| anyhow!("no picked item for this worker"))?
        };
        self.ensure_outcome_feature_run(&item_id)?;
        let item = self.get_item(&item_id)?;
        let plan_id = item
            .plan_path
            .as_deref()
            .map(|path| self.plan_id_for_path(path))
            .transpose()?
            .flatten();
        let ready_before = self.ready_item_ids()?;
        self.conn
            .execute_batch("BEGIN IMMEDIATE; SAVEPOINT settlement_done")?;
        let settlement = (|| -> Result<(bool, Value, String, bool, Value, Option<Value>)> {
            let settled_item = self.get_item(&item_id)?;
            let adopted = self.adopt_ready_item(&item_id)?;
            let materiality = self.settlement_materiality(
                &item_id,
                &args.summary,
                &args.files,
                &args.cmd,
                &args.tests,
                escalation.is_some(),
            );
            let review_required = materiality["effective_review"]["required"]
                .as_bool()
                .unwrap_or(escalation.is_some());
            let settlement_is_retry = matches!(
                settled_item.status,
                ItemStatus::Closed | ItemStatus::ClosedPartial
            );
            let previous_log_id = settlement_is_retry
                .then(|| self.latest_completion_log_id(&item_id))
                .transpose()?
                .flatten();
            let log_id = if let Some(log_id) = previous_log_id {
                log_id
            } else {
                let log_id = self.add_log_entry(LogInput {
                    item_id: &item_id,
                    kind: "completion",
                    summary: &args.summary,
                    files: &args.files,
                    commands: &args.cmd,
                    tests: &args.tests,
                    source: None,
                    profile: args.profile.as_deref(),
                    route_observation: route_observation.as_ref(),
                })?;
                if std::env::var("PLANR_TEST_FAIL_AFTER_COMPLETION_LOG")
                    .ok()
                    .as_deref()
                    == Some("1")
                {
                    bail!("injected_failure: after completion log");
                }
                log_id
            };
            let mut run_transition =
                self.settle_feature_run_outcome(super::execution_run::OutcomeSettlement {
                    item_id: &item_id,
                    summary: &args.summary,
                    materiality: &materiality,
                    escalation: escalation.clone(),
                })?;
            self.close_item_core(&item_id, &args.summary, false)?;
            if std::env::var("PLANR_TEST_FAIL_AFTER_REVIEW_GATE")
                .ok()
                .as_deref()
                == Some("1")
            {
                bail!("injected_failure: after review gate");
            }
            self.persist_settlement_materiality(&item_id, &materiality)?;
            let next = if args.next {
                if run_transition["transition"] == "review_gate" {
                    Some(json!({
                        "item": null,
                        "reason": "review_gate_pending_independent_lease",
                        "review_gate": run_transition["review_gate"],
                    }))
                } else {
                    let compatible_code_ready = self
                        .peek_next_ready_item_filtered(&PickFilter {
                            exclude: None,
                            work_type: Some("code"),
                            plan_path: item.plan_path.as_deref(),
                        })?
                        .is_some();
                    if run_transition["transition"] == "batch_cap_reached" {
                        let plan_id = plan_id.as_deref().ok_or_else(|| {
                            anyhow!("batch_roll_requires_planned_outcome:{item_id}")
                        })?;
                        if compatible_code_ready {
                            let rollover =
                                self.roll_feature_run_batch_value(plan_id, &worker_id())?;
                            run_transition["successor_batch_id"] =
                                rollover["execution_batch"]["id"].clone();
                            run_transition["rollover"] = rollover;
                        }
                    }
                    if compatible_code_ready {
                        Some(self.next_pick_value(None, Some("code"), plan_id.as_deref())?)
                    } else if plan_id.is_some() {
                        let verification_item_id =
                            self.ready_verification_item_for_plan_path(item.plan_path.as_deref())?;
                        Some(json!({
                            "item": null,
                            "reason": "verification_handoff_pending_source_freeze",
                            "verification_item_id": verification_item_id,
                        }))
                    } else {
                        Some(self.next_pick_value(None, Some("code"), None)?)
                    }
                }
            } else {
                None
            };
            Ok((
                adopted,
                materiality,
                log_id,
                review_required,
                run_transition,
                next,
            ))
        })();
        let (adopted, materiality, log_id, review_required, run_transition, mut fused_next) =
            match settlement {
                Ok(value) => {
                    self.conn.execute_batch("RELEASE settlement_done; COMMIT")?;
                    value
                }
                Err(error) => {
                    let _ = self.conn.execute_batch(
                        "ROLLBACK TO settlement_done; RELEASE settlement_done; ROLLBACK",
                    );
                    return Err(error);
                }
            };
        if args.next
            && fused_next.as_ref().and_then(|next| next["reason"].as_str())
                == Some("verification_handoff_pending_source_freeze")
        {
            let plan_id = plan_id
                .as_deref()
                .ok_or_else(|| anyhow!("verification_handoff_requires_plan:{item_id}"))?;
            let source_freeze = self
                .freeze_feature_run_source_value(plan_id)?
                .ok_or_else(|| anyhow!("verification_handoff_source_freeze_missing:{plan_id}"))?;
            let verification_item_id = fused_next
                .as_ref()
                .and_then(|next| next["verification_item_id"].as_str())
                .map(ToOwned::to_owned);
            fused_next = Some(self.canonical_source_frozen_handoff_value(
                plan_id,
                verification_item_id,
                source_freeze,
            )?);
        }
        let extras = self.settlement_extras(
            &item_id,
            &ready_before,
            !args.cmd.is_empty() || !args.tests.is_empty(),
        )?;
        let next = if args.next {
            fused_next
        } else {
            let plan_flag = item
                .plan_path
                .as_deref()
                .and_then(|path| self.plan_id_for_path(path).transpose())
                .transpose()?
                .map(|id| format!(" --plan {id}"))
                .unwrap_or_default();
            let command = if run_transition["transition"] == "review_gate" {
                format!("planr pick{plan_flag} --work-type review --json")
            } else {
                format!("planr pick{plan_flag} --json")
            };
            Some(json!(command))
        };
        let mut human = if run_transition["transition"] == "review_gate" {
            format!(
                "logged and closed {item_id}; opened review gate {}",
                run_transition["review_gate"]["id"]
                    .as_str()
                    .unwrap_or("unknown")
            )
        } else {
            format!("logged and closed {item_id}")
        };
        if adopted {
            human.push_str(" (item was never picked; adopted under this worker)");
        }
        if let Some(next) = &next {
            if let Some(command) = next.as_str() {
                human.push_str(&format!("; next: {command}"));
            } else {
                match next["item"]["id"].as_str() {
                    Some(next_id) => human.push_str(&format!("; picked {next_id}")),
                    None if next["reason"] == "review_gate_pending_independent_lease" => {
                        human.push_str("; review gate awaits an independent reviewer")
                    }
                    None if next["reason"] == "verification_handoff_source_frozen" => {
                        human.push_str("; source frozen for verification handoff")
                    }
                    None if next["reason"] == "nonbinding_final_review_handoff_source_frozen" => {
                        human.push_str("; source frozen for independent final review")
                    }
                    None if next["reason"] == "binding_evidence_obligations_missing" => {
                        human.push_str("; binding Evidence obligations must be materialized")
                    }
                    None => human.push_str("; no ready item"),
                }
            }
        }
        let progress = self.progress_value()?;
        human.push_str(&Self::progress_human(&progress));
        if let Some(reason) = materiality["effective_review"]["reason"].as_str() {
            human.push_str(&format!(
                "\nmateriality: review {} ({reason})",
                if review_required { "required" } else { "none" }
            ));
        }
        human.push_str(&Self::settlement_extras_human(&extras));
        let closed_value = json!(item_id);
        self.emit(
            json!({
                "item": self.get_item(&item_id)?,
                "log_id": log_id,
                "work_packet": run_transition,
                "closed": closed_value,
                "materiality": materiality,
                "unlocked": extras["unlocked"],
                "post_condition": extras["post_condition"],
                "hint": extras["hint"],
                "proof": extras["proof"],
                "next": next,
                "remaining": progress,
            }),
            human,
        )
    }
}
