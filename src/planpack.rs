use crate::util::now_string;
use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use slug::slugify;
use std::{fs, path::Path};

pub fn project_pack_files() -> Vec<(&'static str, String)> {
    vec![
        ("product.md", "# Product\n\n## Purpose\n\nPlanr project context.\n\n## Done Means\n\nGraph state is closed only after evidence is logged.\n".to_string()),
        ("ownership.md", "# Ownership\n\nSQLite owns map state, picks, links, reviews, logs, and closure. Markdown owns durable narrative context.\n".to_string()),
        ("flows.md", "# Flows\n\n1. Capture idea.\n2. Create product plan.\n3. Split build plan.\n4. Build map.\n5. Pick, log, review, and close with evidence.\n".to_string()),
        ("state-ssot.md", "# State Source Of Truth\n\nThe local SQLite database is authoritative for item state, graph links, picks, gates, reviews, logs, and completion.\n".to_string()),
        ("constraints.md", "# Constraints\n\n- Local-first by default.\n- No secrets in logs or context.\n- No provider-specific assumptions in core graph behavior.\n".to_string()),
        ("quality-gates.md", "# Quality Gates\n\n- Plans must have acceptance criteria.\n- Closures require evidence.\n- Review failures create follow-up work instead of closing parent scope.\n".to_string()),
    ]
}

pub fn product_plan_files(
    title: &str,
    platform: Option<&str>,
    ai: bool,
    backend: bool,
) -> Vec<(&'static str, String)> {
    let manifest = json!({
        "title": title,
        "generated_at": now_string(),
        "source_prompt": title,
        "assumptions": [],
        "platform": platform,
        "ai": ai,
        "backend": backend,
        "included_documents": [
            "README.md",
            "PRODUCT_SPEC.md",
            "UX_FLOWS.md",
            "DESIGN_SYSTEM_SPEC.md",
            "TECH_ARCHITECTURE.md",
            "ADRS.md",
            "AI_SPEC.md",
            "SAFETY_PRIVACY_SECURITY.md",
            "API_AND_DATA_MODEL.md",
            "CLIENT_IMPLEMENTATION_SPEC.md",
            "BACKEND_IMPLEMENTATION_SPEC.md",
            "ANALYTICS_OBSERVABILITY_SPEC.md",
            "QA_ACCEPTANCE_TESTS.md",
            "RELEASE_READINESS.md",
            "TASKS.md",
            "REFERENCES.md"
        ]
    });
    let base = format!("# {title}\n\n## Summary\n\n## Goals\n\n## Non-Goals\n\n## Assumptions\n\n");
    vec![
        ("PLANR_MANIFEST.json", serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string())),
        ("README.md", base),
        ("PRODUCT_SPEC.md", "# Product Specification\n\n## Problem\n\n## Users\n\n## Requirements\n\n## Success Criteria\n\n".to_string()),
        ("UX_FLOWS.md", "# UX Flows\n\n## Primary Flow\n\n## Empty States\n\n## Error States\n\n".to_string()),
        ("DESIGN_SYSTEM_SPEC.md", "# Design System\n\n## Principles\n\n## Components\n\n## Accessibility\n\n".to_string()),
        ("TECH_ARCHITECTURE.md", "# Technical Architecture\n\n## Components\n\n## Data Flow\n\n## Failure Modes\n\n".to_string()),
        ("ADRS.md", "# Architecture Decisions\n\n## ADR-001\n\nStatus: proposed\n\nDecision:\n\nConsequences:\n\n".to_string()),
        ("AI_SPEC.md", "# AI Specification\n\n## Model Boundaries\n\n## Prompt Contracts\n\n## Evaluation\n\n".to_string()),
        ("SAFETY_PRIVACY_SECURITY.md", "# Safety Privacy Security\n\n## Data Handling\n\n## Secrets\n\n## Abuse Cases\n\n".to_string()),
        ("API_AND_DATA_MODEL.md", "# API And Data Model\n\n## Objects\n\n## Commands\n\n## Events\n\n".to_string()),
        ("CLIENT_IMPLEMENTATION_SPEC.md", "# Client Implementation\n\n## CLI\n\n## MCP\n\n## UI\n\n".to_string()),
        ("BACKEND_IMPLEMENTATION_SPEC.md", "# Backend Implementation\n\n## Storage\n\n## Services\n\n## Tests\n\n".to_string()),
        ("ANALYTICS_OBSERVABILITY_SPEC.md", "# Analytics Observability\n\n## Events\n\n## Diagnostics\n\n## Privacy\n\n".to_string()),
        ("QA_ACCEPTANCE_TESTS.md", "# QA Acceptance Tests\n\n## Acceptance\n\n## Regression\n\n## Manual Scenarios\n\n".to_string()),
        ("RELEASE_READINESS.md", "# Release Readiness\n\n## Packaging\n\n## Documentation\n\n## Verification\n\n".to_string()),
        ("TASKS.md", "# Tasks\n\n### TASK-001: Build first slice\n\nGoal:\nImplement the first production slice.\n\nAcceptance criteria:\n- The feature is implemented.\n- Verification is logged.\n".to_string()),
        ("REFERENCES.md", "# References\n\n".to_string()),
    ]
}

pub fn build_plan_body(title: &str, source: &str, slice: &str) -> String {
    format!(
        "---\nname: {}\noverview: Build plan for {}.\ntodos:\n  - id: phase-1\n    content: Implement {}\n    status: pending\nisProject: false\nstage: build\nsource_plan: {}\nslice: {}\n---\n\n# {}\n\n## Scope Decision\n\n## Ownership Target\n\n## Existing Leverage\n\n## Phase 1\n\n- [ ] Implement {}\n\n## Out Of Scope\n\n## Verification\n\n## Acceptance Criteria\n\n",
        slugify(title), title, slice, source, slice, title, slice
    )
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

pub fn extract_work_specs(path: &Path) -> Result<Vec<(String, String)>> {
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

fn extract_specs_from_text(text: &str) -> Vec<(String, String)> {
    let mut specs = Vec::new();
    let lines: Vec<_> = text.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if let Some(title) = line.strip_prefix("### ") {
            let title = title
                .split_once(':')
                .map(|(_, rest)| rest.trim())
                .unwrap_or(title)
                .trim();
            let desc = lines
                .iter()
                .skip(idx + 1)
                .take_while(|l| !l.starts_with("### "))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            specs.push((title.to_string(), desc.trim().to_string()));
        } else if let Some(title) = line.trim().strip_prefix("- [ ] ") {
            specs.push((
                title.trim().to_string(),
                format!("Complete checklist item: {}", title.trim()),
            ));
        }
    }
    specs
}
