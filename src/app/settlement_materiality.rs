use crate::usage_policy::{
    ChangeKind, ChangeSummary, MaterialityPolicy, MaterialityTrigger, PolicyLoad,
    ReviewRequirement, RiskLevel, classify_materiality, load_policy,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(super) struct SettlementMaterialityInput<'a> {
    pub(super) root: &'a Path,
    pub(super) item_id: &'a str,
    pub(super) summary: &'a str,
    pub(super) files: &'a [String],
    pub(super) commands: &'a [String],
    pub(super) tests: &'a [String],
    pub(super) explicit_review: bool,
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
    root: &'a Path,
    item_id: &'a str,
    summary: &'a str,
    files: &'a [String],
    commands: &'a [String],
    tests: &'a [String],
    explicit_review: bool,
    reason: &'a str,
    diagnostics: Option<String>,
}

pub(super) fn settlement_materiality_value(input: SettlementMaterialityInput<'_>) -> Value {
    match load_policy(input.root) {
        PolicyLoad::Loaded(policy) => materiality_value(MaterialityValueInput {
            root: input.root,
            policy: &policy.materiality,
            policy_id: &policy.id,
            policy_version: &policy.version,
            item_id: input.item_id,
            summary: input.summary,
            files: input.files,
            commands: input.commands,
            tests: input.tests,
            explicit_review: input.explicit_review,
        }),
        PolicyLoad::Missing => unavailable_policy_materiality(UnavailablePolicyMaterialityInput {
            root: input.root,
            item_id: input.item_id,
            summary: input.summary,
            files: input.files,
            commands: input.commands,
            tests: input.tests,
            explicit_review: input.explicit_review,
            reason: "missing",
            diagnostics: None,
        }),
        PolicyLoad::Invalid(diagnostics) => {
            unavailable_policy_materiality(UnavailablePolicyMaterialityInput {
                root: input.root,
                item_id: input.item_id,
                summary: input.summary,
                files: input.files,
                commands: input.commands,
                tests: input.tests,
                explicit_review: input.explicit_review,
                reason: "invalid",
                diagnostics: Some(diagnostics.to_string()),
            })
        }
    }
}

pub(super) fn unique_files(files: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    files
        .iter()
        .map(|file| file.trim())
        .filter(|file| !file.is_empty())
        .filter(|file| seen.insert((*file).to_string()))
        .map(ToOwned::to_owned)
        .collect()
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
    let mut reasons = Vec::new();
    if policy_loaded
        && files.is_empty()
        && (!summary.trim().is_empty() || !commands.is_empty() || !tests.is_empty())
    {
        reasons.push("missing_changed_files".to_string());
    }
    let line_evidence = git_changed_lines(root, files);
    reasons.extend(line_evidence.reasons);
    let kind = classify_change_kind(files);
    let triggers = infer_materiality_triggers(files);
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
            "files": input.files,
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

fn unavailable_policy_materiality(input: UnavailablePolicyMaterialityInput<'_>) -> Value {
    let (change, mut reasons) = derive_change_summary(
        input.root,
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
            "files": input.files,
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
