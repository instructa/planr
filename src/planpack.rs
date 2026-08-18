//! Markdown plan package parsing and templates.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path};

mod scaffold;

pub use scaffold::{build_plan_body, product_plan_files, project_pack_files};

/// One authored acceptance criterion from build-plan frontmatter.
///
/// This is the canonical criterion identity contract. Markdown acceptance
/// prose is narrative only, and downstream Evidence code must join obligations
/// to this checked list instead of discovering criterion identities elsewhere.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPlanCriterion {
    pub id: String,
    pub title: String,
}

/// Decode and validate the closed build-plan `criteria` contract.
///
/// The caller receives every problem in one pass so `plan check` can present
/// an actionable repair list without accepting partial or duplicate identity
/// sets. Values are validated, never normalized or inferred.
pub fn build_plan_criteria(
    frontmatter: &Value,
) -> std::result::Result<Vec<BuildPlanCriterion>, Vec<String>> {
    let Some(raw_criteria) = frontmatter.get("criteria") else {
        return Err(vec![
            "frontmatter `criteria` must be a non-empty list of `{id, title}` objects".to_string(),
        ]);
    };
    let Some(raw_criteria) = raw_criteria.as_array() else {
        return Err(vec![
            "frontmatter `criteria` must be a non-empty list of `{id, title}` objects".to_string(),
        ]);
    };
    if raw_criteria.is_empty() {
        return Err(vec!["frontmatter `criteria` must not be empty".to_string()]);
    }

    let mut criteria = Vec::with_capacity(raw_criteria.len());
    let mut seen_ids = HashSet::with_capacity(raw_criteria.len());
    let mut problems = Vec::new();
    for (index, value) in raw_criteria.iter().enumerate() {
        let criterion = match serde_json::from_value::<BuildPlanCriterion>(value.clone()) {
            Ok(criterion) => criterion,
            Err(error) => {
                problems.push(format!(
                    "frontmatter `criteria[{index}]` must contain only string `id` and `title` fields: {error}"
                ));
                continue;
            }
        };
        if !valid_criterion_id(&criterion.id) {
            problems.push(format!(
                "frontmatter `criteria[{index}].id` must match `[A-Za-z0-9][A-Za-z0-9._:-]*`"
            ));
        }
        if criterion.title.trim().is_empty() {
            problems.push(format!(
                "frontmatter `criteria[{index}].title` must not be empty"
            ));
        }
        if !seen_ids.insert(criterion.id.clone()) {
            problems.push(format!(
                "frontmatter criterion id `{}` is duplicated",
                criterion.id
            ));
        }
        criteria.push(criterion);
    }

    if problems.is_empty() {
        Ok(criteria)
    } else {
        Err(problems)
    }
}

fn valid_criterion_id(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
}

pub const BUILD_PLAN_REQUIRED_SECTIONS: &[&str] =
    &["Scope Decision", "Verification", "Acceptance Criteria"];
pub const PRODUCT_PLAN_REQUIRED_SECTIONS: &[&str] =
    &["Problem", "Requirements", "Success Criteria"];

/// Report required `##` sections that are missing or have no body content.
/// Sub-headings (`###`) and list items count as content; another `##` or a
/// top-level `#` heading ends a section.
/// Returns (section name, "missing" | "empty") for each required section that
/// has no heading or no body content.
pub fn unfilled_required_sections(text: &str, required: &[&str]) -> Vec<(String, &'static str)> {
    use std::collections::HashSet;
    let mut current: Option<String> = None;
    let mut seen: HashSet<String> = HashSet::new();
    let mut filled: HashSet<String> = HashSet::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let name = heading.trim().to_string();
            seen.insert(name.clone());
            current = Some(name);
            continue;
        }
        if line.starts_with("# ") {
            current = None;
            continue;
        }
        if let Some(section) = &current {
            if !line.trim().is_empty() {
                filled.insert(section.clone());
            }
        }
    }
    required
        .iter()
        .filter_map(|name| {
            if !seen.contains(*name) {
                Some((name.to_string(), "missing"))
            } else if !filled.contains(*name) {
                Some((name.to_string(), "empty"))
            } else {
                None
            }
        })
        .collect()
}

pub fn parse_plan_metadata(path: &Path) -> (Value, String) {
    let target = if path.is_dir() {
        path.join("README.md")
    } else {
        path.to_path_buf()
    };
    let Ok(text) = fs::read_to_string(target) else {
        return (json!({}), "ok".to_string());
    };
    if !text.starts_with("---\n") {
        return (json!({}), "ok".to_string());
    }
    let Some(rest) = text.strip_prefix("---\n") else {
        return (json!({}), "ok".to_string());
    };
    let Some((yaml, _body)) = rest.split_once("\n---") else {
        return (
            json!({"error": "unterminated frontmatter"}),
            "parse_error".to_string(),
        );
    };
    match serde_yaml::from_str::<Value>(yaml) {
        Ok(value) => (value, "ok".to_string()),
        Err(err) => (json!({"error": err.to_string()}), "parse_error".to_string()),
    }
}

pub fn hash_path(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            if entry.is_file() {
                hasher.update(fs::read(&entry)?);
            }
        }
    } else if path.exists() {
        hasher.update(fs::read(path)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn plan_search_body(path: &Path) -> Result<String> {
    let mut body = String::new();
    if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            if entry.extension().and_then(|s| s.to_str()) == Some("md")
                || entry.file_name().and_then(|s| s.to_str()) == Some("PLANR_MANIFEST.json")
            {
                body.push_str(&fs::read_to_string(entry).unwrap_or_default());
                body.push('\n');
            }
        }
    } else if path.exists() {
        body.push_str(&fs::read_to_string(path).unwrap_or_default());
    }
    Ok(body)
}

/// Detects a task list that `map build` would turn into a single coarse
/// item: either no work specs at all, or only the scaffold's placeholder
/// ("Build first slice" in directory plans, "Implement <slice>" in build
/// plans). Returns the reason, or None when the list is expanded. Lives
/// here because the scaffold templates above define the placeholders.
pub fn scaffold_placeholder_state(path: &Path) -> Result<Option<&'static str>> {
    let specs = extract_work_specs(path)?;
    Ok(match specs.as_slice() {
        [] => Some("the task list has no work specs"),
        [spec] if spec.title == "Build first slice" || spec.title.starts_with("Implement ") => {
            Some("the task list still contains only the scaffold placeholder")
        }
        _ => None,
    })
}

/// One task from a plan's work list. `work_type` comes from an optional
/// annotation — `### TASK-001 (frontend): ...` or `- [ ] (frontend) ...`
/// — so plans can declare the use case where the task is written and
/// `map build` seeds routed items directly (no post-build retag).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkSpec {
    pub title: String,
    pub description: String,
    pub work_type: Option<String>,
}

pub fn extract_work_specs(path: &Path) -> Result<Vec<WorkSpec>> {
    let mut specs = Vec::new();
    if path.is_dir() {
        let task_file = path.join("TASKS.md");
        if task_file.exists() {
            specs.extend(extract_specs_from_text(&fs::read_to_string(task_file)?));
        }
    } else if path.exists() {
        specs.extend(extract_specs_from_text(&fs::read_to_string(path)?));
    }
    Ok(specs)
}

fn extract_specs_from_text(text: &str) -> Vec<WorkSpec> {
    let mut specs = Vec::new();
    let lines: Vec<_> = text.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if let Some(heading) = line.strip_prefix("### ") {
            // `### TASK-001 (frontend): Title` — the annotation lives in
            // the pre-colon part so the title stays clean.
            let (prefix, title) = match heading.split_once(':') {
                Some((prefix, rest)) => (prefix, rest.trim()),
                None => ("", heading.trim()),
            };
            let desc = lines
                .iter()
                .skip(idx + 1)
                .take_while(|l| !l.starts_with("### "))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            specs.push(WorkSpec {
                title: title.to_string(),
                description: desc.trim().to_string(),
                work_type: annotation(prefix),
            });
        } else if let Some(rest) = line.trim().strip_prefix("- [ ] ") {
            let (work_type, title) = match leading_annotation(rest.trim()) {
                Some((work_type, title)) => (Some(work_type), title),
                None => (None, rest.trim().to_string()),
            };
            specs.push(WorkSpec {
                description: format!("Complete checklist item: {title}"),
                title,
                work_type,
            });
        }
    }
    specs
}

/// A `(work-type)` token anywhere in a task-heading prefix. Single
/// identifier-like tokens only, so prose parentheticals never match.
fn annotation(prefix: &str) -> Option<String> {
    let start = prefix.find('(')?;
    let end = prefix[start..].find(')')? + start;
    work_type_token(&prefix[start + 1..end])
}

/// `(work-type) rest of title` at the start of a checklist item.
fn leading_annotation(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix('(')?;
    let (token, title) = rest.split_once(')')?;
    let work_type = work_type_token(token)?;
    let title = title.trim();
    (!title.is_empty()).then(|| (work_type, title.to_string()))
}

fn work_type_token(token: &str) -> Option<String> {
    let token = token.trim();
    let valid = !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| token.to_string())
}
