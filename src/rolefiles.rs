//! Static workflow role files and skills installed for supported hosts.
//! Model selection and generated routing artifacts are owned by external tools.

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
            "grok" => ".grok/config.toml",
            _ => return paths,
        });
    }
    if client == "grok" {
        paths.extend(grok_workflow_assets().iter().map(|(path, _)| *path));
        return paths;
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

macro_rules! grok_asset {
    ($relative:literal) => {
        (
            concat!(".grok/", $relative),
            include_str!(concat!("../plugins/planr/", $relative)),
        )
    };
}

/// Canonical repository-local Grok workflow payload.
///
/// Both preview and writes consume this mapping. Content is compiled directly
/// from the packaged Planr plugin tree so Grok never owns a forked skill or
/// agent body.
pub fn grok_workflow_assets() -> &'static [(&'static str, &'static str)] {
    &[
        grok_asset!("agents/planr-reviewer.md"),
        grok_asset!("agents/planr-worker.md"),
        grok_asset!("skills/planr-goal/SKILL.md"),
        grok_asset!("skills/planr-loop/SKILL.md"),
        grok_asset!("skills/planr-loop/agents/planr-reviewer.md"),
        grok_asset!("skills/planr-loop/agents/planr-worker.md"),
        grok_asset!("skills/planr-loop/references/host-dispatch.md"),
        grok_asset!("skills/planr-loop/references/recovery-and-verification.md"),
        grok_asset!("skills/planr-plan/SKILL.md"),
        grok_asset!("skills/planr-review/SKILL.md"),
        grok_asset!("skills/planr-status/SKILL.md"),
        grok_asset!("skills/planr-summary/SKILL.md"),
        grok_asset!("skills/planr-task-graph/SKILL.md"),
        grok_asset!("skills/planr-verify-web/SKILL.md"),
        grok_asset!("skills/planr-work/SKILL.md"),
        grok_asset!("skills/planr/SKILL.md"),
    ]
}

macro_rules! cursor_skill {
    ($name:literal) => {
        (
            concat!(".cursor/skills/", $name, "/SKILL.md"),
            include_str!(concat!("../plugins/planr/skills/", $name, "/SKILL.md")),
        )
    };
}

macro_rules! cursor_skill_file {
    ($relative:literal) => {
        (
            concat!(".cursor/skills/", $relative),
            include_str!(concat!("../plugins/planr/skills/", $relative)),
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
        cursor_skill_file!("planr-loop/references/host-dispatch.md"),
        cursor_skill_file!("planr-loop/references/recovery-and-verification.md"),
        cursor_skill!("planr-verify-web"),
        cursor_skill!("planr-task-graph"),
        cursor_skill!("planr-plan"),
        cursor_skill!("planr-work"),
        cursor_skill!("planr-review"),
        cursor_skill!("planr-status"),
        cursor_skill!("planr-summary"),
    ]
}

#[cfg(test)]
mod grok_tests {
    use super::{grok_workflow_assets, install_artifact_paths};
    use std::collections::BTreeSet;

    #[test]
    fn artifact_preview_is_the_workflow_write_source_of_truth() {
        let assets = grok_workflow_assets();
        let asset_paths: Vec<_> = assets.iter().map(|(path, _)| *path).collect();
        let preview = install_artifact_paths("grok", true, true);
        assert_eq!(preview[0], ".grok/config.toml");
        assert_eq!(&preview[1..], asset_paths);
        assert_eq!(
            install_artifact_paths("grok", false, false),
            asset_paths,
            "no-mcp and no-hook mode must still install native workflow assets"
        );
        assert!(
            preview
                .iter()
                .all(|path| !path.starts_with(".grok/plugins/")),
            "v1 must not emit a disabled project plugin"
        );
        let unique: BTreeSet<_> = preview.iter().copied().collect();
        assert_eq!(unique.len(), preview.len(), "artifact paths must be unique");
    }
}
