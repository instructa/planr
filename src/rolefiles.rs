//! Host role file boundary for Claude/Cursor static roles and Cursor skills.
//! Native Codex roles are owned exclusively by preset application.

use crate::agents::AgentProfile;

/// Subagent role files installed into a project so loop dispatches work
/// without the host plugin system (Codex plugins cannot register agents).
pub fn agent_roles(client: &str) -> &'static [(&'static str, &'static str)] {
    match client {
        "codex" => &[],
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

/// Renders a Claude Code agent markdown role pinned to a registry
/// profile. Claude frontmatter takes model aliases or full ids plus an
/// `effort` level; note the `CLAUDE_CODE_SUBAGENT_MODEL` env var preempts
/// any frontmatter pin.
pub fn render_claude_role(
    static_content: &str,
    profile_id: &str,
    profile: &AgentProfile,
    evidence_note: Option<&str>,
) -> Option<String> {
    render_markdown_role(
        static_content,
        profile_id,
        &profile.model,
        profile.effort.as_deref(),
        evidence_note,
    )
}

/// Renders a Cursor agent markdown role pinned to a registry profile.
/// Cursor frontmatter supports `model:` only (no effort field); plan,
/// admin policy, or Max Mode can still override the pin silently.
pub fn render_cursor_role(
    static_content: &str,
    profile_id: &str,
    profile: &AgentProfile,
    evidence_note: Option<&str>,
) -> Option<String> {
    render_markdown_role(
        static_content,
        profile_id,
        &profile.model,
        None,
        evidence_note,
    )
}

/// Shared markdown renderer: keeps the static frontmatter fields and body
/// verbatim, drops the static cost-tiering comments and model/effort
/// lines, and appends the generated-from header plus the pinned values.
fn render_markdown_role(
    static_content: &str,
    profile_id: &str,
    model: &str,
    effort: Option<&str>,
    evidence_note: Option<&str>,
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
    if let Some(note) = evidence_note {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(note);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{GENERATED_FROM_HEADER, agent_roles, render_claude_role, render_cursor_role};
    use crate::agents::AgentProfile;

    fn profile(model: &str, effort: Option<&str>) -> AgentProfile {
        AgentProfile {
            client: "codex".to_string(),
            model: model.to_string(),
            agent_type: None,
            effort: effort.map(ToOwned::to_owned),
            cost_tier: Some("standard".to_string()),
            capabilities: Vec::new(),
            skill: None,
            notes: None,
        }
    }

    fn frontmatter_and_body(content: &str) -> (Vec<&str>, &str) {
        let rest = content.strip_prefix("---\n").unwrap();
        let (frontmatter, body) = rest.split_once("\n---\n").unwrap();
        (frontmatter.lines().collect(), body)
    }

    #[test]
    fn claude_render_pins_model_and_effort_and_keeps_body() {
        let (_, static_content) = agent_roles("claude")[0];
        let rendered = render_claude_role(
            static_content,
            "gpt55-coder",
            &profile("opus", Some("high")),
            None,
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
        let rendered = render_cursor_role(
            static_content,
            "driver",
            &profile("fable-5", Some("high")),
            None,
        )
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
    fn markdown_render_appends_evidence_note_to_the_body() {
        let (_, static_content) = agent_roles("cursor")[0];
        let rendered = render_cursor_role(
            static_content,
            "gpt55-coder",
            &profile("gpt-5.5", None),
            Some("Report with `--profile gpt55-coder`."),
        )
        .unwrap();
        let (_, body) = frontmatter_and_body(&rendered);
        assert!(body.ends_with("Report with `--profile gpt55-coder`.\n"));
    }

    #[test]
    fn markdown_render_rejects_content_without_frontmatter() {
        assert!(
            render_cursor_role("no frontmatter here", "p", &profile("m", None), None).is_none()
        );
    }
}
