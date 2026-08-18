//! Plan package scaffold and template generation.

use serde_json::json;
use slug::slugify;
use time::OffsetDateTime;

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

/// Quote a free-text value as a YAML scalar. A JSON string is a valid YAML
/// double-quoted scalar, which keeps colons, quotes, and hashes parseable.
fn yaml_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

pub fn build_plan_body(title: &str, source: &str, slice: &str) -> String {
    let criterion_id = format!("criterion-{}", slugify(slice));
    format!(
        "---\nname: {}\noverview: {}\ntodos:\n  - id: phase-1\n    content: {}\n    status: pending\nisProject: false\nstage: build\nsource_plan: {}\nslice: {}\ncriteria:\n  - id: {}\n    title: {}\n---\n\n# {}\n\n## Scope Decision\n\n## Ownership Target\n\n## Existing Leverage\n\n## Phase 1\n\n- [ ] Implement {}\n\n## Out Of Scope\n\n## Verification\n\n## Acceptance Criteria\n\n",
        slugify(title),
        yaml_quote(&format!("Build plan for {title}.")),
        yaml_quote(&format!("Implement {slice}")),
        source,
        yaml_quote(slice),
        criterion_id,
        yaml_quote(slice),
        title,
        slice
    )
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown-time".to_string())
}
