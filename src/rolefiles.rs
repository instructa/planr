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
    if client == "pi" {
        return pi_workflow_assets().iter().map(|(path, _)| *path).collect();
    }
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

macro_rules! pi_skill {
    ($relative:literal) => {
        (
            concat!(".pi/skills/", $relative),
            include_str!(concat!("../plugins/planr/skills/", $relative)),
        )
    };
}

/// Canonical repository-local Pi workflow payload.
///
/// Pi core discovers the skill tree directly. The two Pi-specific roles are
/// inert unless the optional `pi-subagents` package is installed.
pub fn pi_workflow_assets() -> &'static [(&'static str, &'static str)] {
    &[
        (
            ".pi/agents/planr-reviewer.md",
            include_str!("../plugins/planr/agents/pi/planr-reviewer.md"),
        ),
        (
            ".pi/agents/planr-worker.md",
            include_str!("../plugins/planr/agents/pi/planr-worker.md"),
        ),
        pi_skill!("planr-goal/SKILL.md"),
        pi_skill!("planr-loop/SKILL.md"),
        pi_skill!("planr-loop/agents/planr-reviewer.md"),
        pi_skill!("planr-loop/agents/planr-worker.md"),
        pi_skill!("planr-loop/references/host-dispatch.md"),
        pi_skill!("planr-loop/references/recovery-and-verification.md"),
        pi_skill!("planr-plan/SKILL.md"),
        pi_skill!("planr-review/SKILL.md"),
        pi_skill!("planr-status/SKILL.md"),
        pi_skill!("planr-summary/SKILL.md"),
        pi_skill!("planr-task-graph/SKILL.md"),
        pi_skill!("planr-verify-web/SKILL.md"),
        pi_skill!("planr-work/SKILL.md"),
        pi_skill!("planr/SKILL.md"),
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
mod host_asset_tests {
    use super::{grok_workflow_assets, install_artifact_paths, pi_workflow_assets};
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

    #[test]
    fn pi_preview_and_write_share_native_project_assets() {
        let assets = pi_workflow_assets();
        let asset_paths: Vec<_> = assets.iter().map(|(path, _)| *path).collect();
        assert_eq!(asset_paths.len(), 16);
        assert_eq!(install_artifact_paths("pi", true, true), asset_paths);
        assert_eq!(install_artifact_paths("pi", false, false), asset_paths);
        assert!(asset_paths.iter().all(|path| path.starts_with(".pi/")));
        assert!(
            asset_paths.iter().all(|path| {
                !path.contains("/extensions/")
                    && !path.ends_with("settings.json")
                    && !path.contains("mcp")
                    && !path.contains("hooks")
            }),
            "Pi V1 must emit only native skills and optional subagent roles"
        );
        let unique: BTreeSet<_> = asset_paths.iter().copied().collect();
        assert_eq!(unique.len(), asset_paths.len());
    }

    #[test]
    fn pi_roles_use_pi_subagents_frontmatter_without_model_pins() {
        for (path, skill, mutable) in [
            (".pi/agents/planr-worker.md", "skills: planr-work", true),
            (
                ".pi/agents/planr-reviewer.md",
                "skills: planr-review",
                false,
            ),
        ] {
            let content = pi_workflow_assets()
                .iter()
                .find_map(|(candidate, content)| (*candidate == path).then_some(*content))
                .unwrap_or_else(|| panic!("missing Pi role {path}"));
            for required in [
                "systemPromptMode: replace",
                "inheritProjectContext: true",
                "inheritSkills: false",
                skill,
            ] {
                assert!(content.contains(required), "{path} missing {required}");
            }
            for forbidden in [
                "\nmodel:",
                "\nthinking:",
                "\neffort:",
                "\nfallbackModels:",
                "\nextensions:",
            ] {
                assert!(
                    !content.contains(forbidden),
                    "{path} must leave model and extension policy to Pi"
                );
            }
            if mutable {
                assert!(!content.contains("acceptanceRole:"));
                assert!(!content.contains("completionGuard:"));
            } else {
                assert!(content.contains("acceptanceRole: read-only"));
                assert!(content.contains("completionGuard: false"));
            }
            assert_eq!(
                content.contains("tools: read, grep, find, ls, bash, edit, write"),
                mutable,
                "only the Pi worker may receive mutation tools"
            );
        }
    }
}
