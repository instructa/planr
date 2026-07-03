//! Host role file boundary: owns the shipped static subagent role files
//! (Codex TOML, Claude/Cursor markdown, Cursor skills) and the pure
//! renderers that re-pin those files from `.planr/agents.toml` profiles.
//! Renderers never call providers and never fail installs — anything
//! unrenderable falls back to the static text.

use crate::agents::AgentProfile;

/// Subagent role files installed into a project so loop dispatches work
/// without the host plugin system (Codex plugins cannot register agents).
pub fn agent_roles(client: &str) -> &'static [(&'static str, &'static str)] {
    match client {
        "codex" => &[
            (
                ".codex/agents/planr-worker.toml",
                include_str!("../plugins/planr/skills/planr-loop/agents/planr-worker.toml"),
            ),
            (
                ".codex/agents/planr-reviewer.toml",
                include_str!("../plugins/planr/skills/planr-loop/agents/planr-reviewer.toml"),
            ),
        ],
        "claude" => &[
            (
                ".claude/agents/planr-worker.md",
                include_str!("../plugins/planr/agents/planr-worker.md"),
            ),
            (
                ".claude/agents/planr-reviewer.md",
                include_str!("../plugins/planr/agents/planr-reviewer.md"),
            ),
        ],
        "cursor" => &[
            (
                ".cursor/agents/planr-worker.md",
                include_str!("../plugins/planr/skills/planr-loop/agents/planr-worker.md"),
            ),
            (
                ".cursor/agents/planr-reviewer.md",
                include_str!("../plugins/planr/skills/planr-loop/agents/planr-reviewer.md"),
            ),
        ],
        _ => &[],
    }
}

macro_rules! cursor_skill {
    ($name:literal) => {
        (
            concat!(".cursor/skills/", $name, "/SKILL.md"),
            include_str!(concat!("../plugins/planr/skills/", $name, "/SKILL.md")),
        )
    };
}

/// Skills installed into `.cursor/skills/` by `planr install cursor`, so the
/// full skill set works in one command without waiting on the marketplace
/// listing. Cursor loads project skills from `.cursor/skills/<name>/SKILL.md`.
pub fn cursor_skills() -> &'static [(&'static str, &'static str)] {
    &[
        cursor_skill!("planr"),
        cursor_skill!("planr-goal"),
        cursor_skill!("planr-loop"),
        cursor_skill!("planr-verify-web"),
        cursor_skill!("planr-task-graph"),
        cursor_skill!("planr-plan"),
        cursor_skill!("planr-work"),
        cursor_skill!("planr-review"),
        cursor_skill!("planr-status"),
        cursor_skill!("planr-summary"),
    ]
}

/// Marks a role file as rendered from the registry. The run-audit and
/// doctor slice key off this exact text to tell generated artifacts from
/// hand-maintained ones — change it only together with them.
pub const GENERATED_FROM_HEADER: &str =
    "generated from .planr/agents.toml (planr install --force to re-render)";

/// Renders a Codex agent role TOML pinned to a registry profile, reusing
/// the shipped static role file for name, description, prompt, and skill
/// wiring. Codex field names are strict: `developer_instructions` (a role
/// file without it is silently ignored, openai/codex#26868 family) and
/// `model_reasoning_effort` — not `instructions`/`reasoning_effort`.
/// Returns None when the static content cannot be understood; callers
/// fall back to the static text, never fail.
pub fn render_codex_role(
    static_content: &str,
    profile_id: &str,
    profile: &AgentProfile,
) -> Option<String> {
    let parsed: toml::Value = toml::from_str(static_content).ok()?;
    let table = parsed.as_table()?;
    let name = table.get("name")?.as_str()?;
    let description = table.get("description")?.as_str()?;
    let developer_instructions = table.get("developer_instructions")?.as_str()?;
    if developer_instructions.contains("\"\"\"") || !developer_instructions.ends_with('\n') {
        return None;
    }
    let mut out = format!(
        "# {GENERATED_FROM_HEADER}\n# profile: {profile_id}\nname = {}\ndescription = {}\nmodel = {}\n",
        toml_string(name),
        toml_string(description),
        toml_string(&profile.model),
    );
    if let Some(effort) = profile.effort.as_deref() {
        out.push_str(&format!(
            "model_reasoning_effort = {}\n",
            toml_string(effort)
        ));
    }
    if let Some(sandbox_mode) = table.get("sandbox_mode").and_then(toml::Value::as_str) {
        out.push_str(&format!("sandbox_mode = {}\n", toml_string(sandbox_mode)));
    }
    out.push_str(&format!(
        "\ndeveloper_instructions = \"\"\"\n{developer_instructions}\"\"\"\n"
    ));
    if let Some(configs) = table
        .get("skills")
        .and_then(|skills| skills.get("config"))
        .and_then(toml::Value::as_array)
    {
        for config in configs {
            let path = config.get("path").and_then(toml::Value::as_str)?;
            let enabled = config
                .get("enabled")
                .and_then(toml::Value::as_bool)
                .unwrap_or(true);
            out.push_str(&format!(
                "\n[[skills.config]]\npath = {}\nenabled = {enabled}\n",
                toml_string(path)
            ));
        }
    }
    Some(out)
}

/// Renders a Claude Code agent markdown role pinned to a registry
/// profile. Claude frontmatter takes model aliases or full ids plus an
/// `effort` level; note the `CLAUDE_CODE_SUBAGENT_MODEL` env var preempts
/// any frontmatter pin.
pub fn render_claude_role(
    static_content: &str,
    profile_id: &str,
    profile: &AgentProfile,
) -> Option<String> {
    render_markdown_role(
        static_content,
        profile_id,
        &profile.model,
        profile.effort.as_deref(),
    )
}

/// Renders a Cursor agent markdown role pinned to a registry profile.
/// Cursor frontmatter supports `model:` only (no effort field); plan,
/// admin policy, or Max Mode can still override the pin silently.
pub fn render_cursor_role(
    static_content: &str,
    profile_id: &str,
    profile: &AgentProfile,
) -> Option<String> {
    render_markdown_role(static_content, profile_id, &profile.model, None)
}

/// Shared markdown renderer: keeps the static frontmatter fields and body
/// verbatim, drops the static cost-tiering comments and model/effort
/// lines, and appends the generated-from header plus the pinned values.
fn render_markdown_role(
    static_content: &str,
    profile_id: &str,
    model: &str,
    effort: Option<&str>,
) -> Option<String> {
    let rest = static_content.strip_prefix("---\n")?;
    let (frontmatter, body) = rest.split_once("\n---\n")?;
    let mut out = String::from("---\n");
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#')
            || trimmed.starts_with("model:")
            || trimmed.starts_with("effort:")
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!(
        "# {GENERATED_FROM_HEADER}\n# profile: {profile_id}\nmodel: {model}\n"
    ));
    if let Some(effort) = effort {
        out.push_str(&format!("effort: {effort}\n"));
    }
    out.push_str("---\n");
    out.push_str(body);
    Some(out)
}

/// TOML-escapes a string as a quoted basic string.
fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        GENERATED_FROM_HEADER, agent_roles, render_claude_role, render_codex_role,
        render_cursor_role,
    };
    use crate::agents::AgentProfile;

    fn profile(model: &str, effort: Option<&str>) -> AgentProfile {
        AgentProfile {
            client: "codex".to_string(),
            model: model.to_string(),
            effort: effort.map(ToOwned::to_owned),
            cost_tier: Some("standard".to_string()),
            capabilities: Vec::new(),
            notes: None,
        }
    }

    fn frontmatter_and_body(content: &str) -> (Vec<&str>, &str) {
        let rest = content.strip_prefix("---\n").unwrap();
        let (frontmatter, body) = rest.split_once("\n---\n").unwrap();
        (frontmatter.lines().collect(), body)
    }

    #[test]
    fn codex_render_round_trips_profile_and_keeps_prompt() {
        let (_, static_content) = agent_roles("codex")[0];
        let rendered = render_codex_role(
            static_content,
            "gpt55-coder",
            &profile("gpt-5.5", Some("xhigh")),
        )
        .unwrap();
        assert!(rendered.starts_with(&format!(
            "# {GENERATED_FROM_HEADER}\n# profile: gpt55-coder\n"
        )));
        let parsed: toml::Value = toml::from_str(&rendered).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(parsed["model_reasoning_effort"].as_str(), Some("xhigh"));
        let static_parsed: toml::Value = toml::from_str(static_content).unwrap();
        for field in ["name", "description", "developer_instructions"] {
            assert_eq!(
                parsed[field].as_str(),
                static_parsed[field].as_str(),
                "field {field} drifted from the static role"
            );
        }
        assert_eq!(
            parsed["skills"]["config"][0]["path"].as_str(),
            static_parsed["skills"]["config"][0]["path"].as_str()
        );
        assert_eq!(
            parsed["skills"]["config"][0]["enabled"].as_bool(),
            Some(true)
        );
    }

    /// Regression guard: a Codex role file without `developer_instructions`
    /// is silently ignored by Codex, so every render must carry it.
    #[test]
    fn codex_render_always_contains_developer_instructions() {
        for (_, static_content) in agent_roles("codex") {
            let rendered =
                render_codex_role(static_content, "p", &profile("gpt-5.5", None)).unwrap();
            let parsed: toml::Value = toml::from_str(&rendered).unwrap();
            assert!(
                !parsed["developer_instructions"]
                    .as_str()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn codex_render_without_effort_omits_the_field_and_keeps_sandbox_mode() {
        let (_, reviewer) = agent_roles("codex")[1];
        let rendered = render_codex_role(reviewer, "driver", &profile("fable-5", None)).unwrap();
        let parsed: toml::Value = toml::from_str(&rendered).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("fable-5"));
        assert!(parsed.get("model_reasoning_effort").is_none());
        // The static reviewer restricts its sandbox; the render must not
        // widen it.
        assert_eq!(parsed["sandbox_mode"].as_str(), Some("workspace-write"));
    }

    #[test]
    fn claude_render_pins_model_and_effort_and_keeps_body() {
        let (_, static_content) = agent_roles("claude")[0];
        let rendered = render_claude_role(
            static_content,
            "gpt55-coder",
            &profile("opus", Some("high")),
        )
        .unwrap();
        let (lines, body) = frontmatter_and_body(&rendered);
        assert!(lines.contains(&"model: opus"));
        assert!(lines.contains(&"effort: high"));
        assert!(lines.contains(&format!("# {GENERATED_FROM_HEADER}").as_str()));
        assert!(lines.contains(&"# profile: gpt55-coder"));
        let (static_lines, static_body) = frontmatter_and_body(static_content);
        assert_eq!(
            body, static_body,
            "prompt body drifted from the static role"
        );
        for kept in ["name: planr-worker", "skills:", "  - planr-work"] {
            assert!(
                lines.contains(&kept) && static_lines.contains(&kept),
                "expected `{kept}` in both static and rendered frontmatter"
            );
        }
        assert!(
            !lines.iter().any(|line| line.contains("Cost tiering")),
            "static cost-tiering comment should be replaced by the generated header"
        );
    }

    #[test]
    fn cursor_render_pins_model_only() {
        let (_, static_content) = agent_roles("cursor")[0];
        let rendered =
            render_cursor_role(static_content, "driver", &profile("fable-5", Some("high")))
                .unwrap();
        let (lines, body) = frontmatter_and_body(&rendered);
        assert!(lines.contains(&"model: fable-5"));
        // Cursor frontmatter has no effort field; the profile's effort must
        // not leak into an unsupported key.
        assert!(!lines.iter().any(|line| line.starts_with("effort:")));
        assert!(!lines.contains(&"model: inherit"));
        let (_, static_body) = frontmatter_and_body(static_content);
        assert_eq!(body, static_body);
    }

    #[test]
    fn markdown_render_rejects_content_without_frontmatter() {
        assert!(render_cursor_role("no frontmatter here", "p", &profile("m", None)).is_none());
    }
}
