//! Host hook installation: `planr install <client>` wires `planr prime`
//! into the host's native hook system by default (`--no-hooks` opts
//! out). Design rules from the product spec: every hook command fails
//! open (`|| true`, short timeout) so a missing or broken planr never
//! blocks a session, existing hook files are merged additively (foreign
//! entries preserved, planr entries never duplicated), and nothing is
//! overwritten destructively.

use super::App;
use anyhow::Result;
use serde_json::{Value, json};
use std::fs;

const PRIME_COMMAND: &str = "planr prime 2>/dev/null || true";
const PRIME_COMMAND_CLAUDE: &str = "planr prime --hook-json 2>/dev/null || true";
const GUARD_RELATIVE: &str = ".cursor/hooks/planr-evidence-guard.sh";

/// Advisory stop-time guard: reminds an ending subagent about held
/// picks that have no completion log yet. Silent otherwise; fails open.
const EVIDENCE_GUARD: &str = r#"#!/bin/bash
# planr evidence guard (advisory): a subagent that stops while holding a
# pick without a completion log gets one follow-up reminder. Never
# blocks; any failure exits silently.
set -u
command -v planr >/dev/null 2>&1 || exit 0
command -v jq >/dev/null 2>&1 || exit 0
held=$(planr --json map show 2>/dev/null | jq -r '
  [.items[]? | select(.status == "picked" or .status == "running")] | .[0].id // empty
' 2>/dev/null)
[ -z "$held" ] && exit 0
logs=$(planr --json log list --item "$held" 2>/dev/null | jq -r '
  [.logs[]? | select(.kind == "completion")] | length
' 2>/dev/null)
if [ "${logs:-0}" = "0" ]; then
  printf '{"followup_message": "planr: %s is still picked with no completion log. Log evidence (planr done %s --summary ... --cmd ...) or release it (planr pick release %s)."}\n' "$held" "$held" "$held"
fi
exit 0
"#;

/// Outcome of a hook install for one client: paths written or extended,
/// plus warnings for files that were left untouched because planr could
/// not merge into them safely.
#[derive(Debug, Default)]
pub(crate) struct HookInstall {
    pub(crate) written: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

impl App {
    /// Writes the hook configuration for one client, merging additively
    /// into existing files. Unparseable or unexpectedly shaped existing
    /// files are never touched — a hook install must not risk a user's
    /// settings file.
    pub(crate) fn install_hooks(&self, client: &str) -> Result<HookInstall> {
        let mut result = HookInstall::default();
        match client {
            "cursor" => {
                let guard_path = self.root.join(GUARD_RELATIVE);
                if !guard_path.exists() {
                    crate::util::write_if_missing(&guard_path, EVIDENCE_GUARD, false)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&guard_path, fs::Permissions::from_mode(0o755))?;
                    }
                    result.written.push(GUARD_RELATIVE.to_string());
                }
                self.merge_hook_file(
                    ".cursor/hooks.json",
                    json!({"version": 1, "hooks": {}}),
                    Some("hooks"),
                    &[
                        (
                            "sessionStart",
                            "planr prime",
                            json!({"command": PRIME_COMMAND, "timeout": 10}),
                        ),
                        (
                            "preCompact",
                            "planr prime",
                            json!({"command": PRIME_COMMAND, "timeout": 10}),
                        ),
                        (
                            "subagentStop",
                            "planr-evidence-guard",
                            json!({"command": GUARD_RELATIVE, "timeout": 10}),
                        ),
                    ],
                    &mut result,
                )?;
            }
            "claude" => {
                self.merge_hook_file(
                    ".claude/settings.json",
                    json!({}),
                    Some("hooks"),
                    &[
                        (
                            "SessionStart",
                            "planr prime",
                            json!({
                                "matcher": "startup|resume|compact",
                                "hooks": [{"type": "command", "command": PRIME_COMMAND_CLAUDE, "timeout": 10}],
                            }),
                        ),
                        (
                            "PreCompact",
                            "planr prime",
                            json!({
                                "hooks": [{"type": "command", "command": PRIME_COMMAND_CLAUDE, "timeout": 10}],
                            }),
                        ),
                    ],
                    &mut result,
                )?;
            }
            "codex" => {
                self.merge_hook_file(
                    ".codex/hooks.json",
                    json!({}),
                    None,
                    &[
                        (
                            "SessionStart",
                            "planr prime",
                            json!({"hooks": [{"type": "command", "command": PRIME_COMMAND, "timeout": 10}]}),
                        ),
                        (
                            "PostCompact",
                            "planr prime",
                            json!({"hooks": [{"type": "command", "command": PRIME_COMMAND, "timeout": 10}]}),
                        ),
                    ],
                    &mut result,
                )?;
                result.warnings.push(
                    "codex requires trusting hooks once: run /hooks in the codex TUI and trust the planr entries".to_string(),
                );
            }
            _ => {}
        }
        Ok(result)
    }

    /// Additive merge into one hook file: load (or start from `default`),
    /// descend into `events_key` when the events live under a wrapper
    /// object, and append each entry unless its marker already exists.
    fn merge_hook_file(
        &self,
        relative: &str,
        default: Value,
        events_key: Option<&str>,
        entries: &[(&str, &str, Value)],
        result: &mut HookInstall,
    ) -> Result<()> {
        let path = self.root.join(relative);
        let mut root = if path.exists() {
            match fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            {
                Some(value) if value.is_object() => value,
                _ => {
                    result.warnings.push(format!(
                        "{relative} exists but is not a JSON object planr can merge into; hooks skipped — add them manually (docs/HOOKS.md)"
                    ));
                    return Ok(());
                }
            }
        } else {
            default
        };
        let events = match events_key {
            Some(key) => {
                let object = root.as_object_mut().expect("checked object above");
                let entry = object.entry(key.to_string()).or_insert_with(|| json!({}));
                match entry.as_object_mut() {
                    Some(events) => events,
                    None => {
                        result.warnings.push(format!(
                            "{relative} has a non-object `{key}` section; hooks skipped — add them manually (docs/HOOKS.md)"
                        ));
                        return Ok(());
                    }
                }
            }
            None => root.as_object_mut().expect("checked object above"),
        };
        let mut changed = false;
        for (event, marker, entry) in entries {
            changed |= push_unique(events, event, marker, entry.clone());
        }
        if changed {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&root)?))?;
            result.written.push(relative.to_string());
        }
        Ok(())
    }
}

/// Appends `entry` to the event's array unless an existing entry already
/// mentions `marker` anywhere in its serialized form — the additive-merge
/// rule that keeps foreign hooks intact and planr hooks unduplicated.
fn push_unique(
    events: &mut serde_json::Map<String, Value>,
    event: &str,
    marker: &str,
    entry: Value,
) -> bool {
    let list = events.entry(event.to_string()).or_insert_with(|| json!([]));
    let Some(list) = list.as_array_mut() else {
        return false;
    };
    if list
        .iter()
        .any(|existing| existing.to_string().contains(marker))
    {
        return false;
    }
    list.push(entry);
    true
}
