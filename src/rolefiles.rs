//! Static workflow role files and skills installed for supported hosts.
//! Model selection and generated routing artifacts are owned by routing bundles.

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

/// Repository-local artifacts reconciled by `planr install` for one client.
/// This is also the dry-run path contract, so preview and writes cannot drift.
pub fn install_artifact_paths(
    client: &str,
    include_mcp: bool,
    include_hooks: bool,
) -> Vec<&'static str> {
    let mut paths = Vec::new();
    if include_mcp {
        paths.push(match client {
            "codex" => ".planr/integrations/codex-mcp.toml",
            "claude" => ".mcp.json",
            "cursor" => ".cursor/mcp.json",
            _ => return paths,
        });
    }
    paths.extend(agent_roles(client).iter().map(|(path, _)| *path));
    if client == "cursor" {
        paths.extend(cursor_skills().iter().map(|(path, _)| *path));
    }
    if include_hooks {
        match client {
            "codex" => paths.push(".codex/hooks.json"),
            "claude" => paths.push(".claude/settings.json"),
            "cursor" => paths.extend([
                ".cursor/hooks.json",
                ".cursor/hooks/planr-evidence-guard.sh",
            ]),
            _ => {}
        }
    }
    paths
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
