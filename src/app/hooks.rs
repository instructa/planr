//! Host hook installation: `planr install <client>` wires `planr prime`
//! into the host's native hook system by default (`--no-hooks` opts
//! out). Design rules from the product spec: every hook command fails
//! open (`|| true`, short timeout) so a missing or broken planr never
//! blocks a session, existing hook files are merged additively (foreign
//! entries preserved, planr entries never duplicated), and nothing is
//! overwritten destructively.

use super::App;
use crate::codex_compat::codex_0145_hook_event_supported;
use anyhow::Result;
use serde_json::{Value, json};
use std::fs;

const PRIME_COMMAND_CODEX: &str = "planr prime 2>/dev/null || true";
const PRIME_COMMAND_CURSOR: &str = "planr prime --cursor-json 2>/dev/null || true";
const PRIME_COMMAND_CLAUDE: &str = "planr prime --hook-json 2>/dev/null || true";
const GUARD_RELATIVE: &str = ".cursor/hooks/planr-evidence-guard.sh";
const CODEX_STOP_RELATIVE: &str = ".codex/hooks/planr-codex-stop.sh";
const CODEX_STOP_SCRIPT: &str = include_str!("../../scripts/hooks/planr-codex-stop.sh");

/// Advisory stop-time guard: reminds an ending subagent about held picks
/// that have no completion log yet. Scoped to the current worker
/// identity when one is set — on shared boards a global "you still owe
/// evidence" message could steer the wrong agent, so without an explicit
/// identity the guard stays silent rather than misleading. JSON is built
/// by jq, never by string interpolation.
const EVIDENCE_GUARD: &str = r#"#!/bin/bash
# planr evidence guard (advisory): a subagent that stops while holding a
# pick without a completion log gets one follow-up reminder. Never
# blocks; any failure exits silently.
set -u
command -v planr >/dev/null 2>&1 || exit 0
command -v jq >/dev/null 2>&1 || exit 0
worker="${PLANR_WORKER_ID:-${PLANR_SESSION_ID:-}}"
[ -z "$worker" ] && exit 0
held=$(planr --json map show 2>/dev/null | jq -r --arg w "$worker" '
  [.items[]? | select((.status == "picked" or .status == "running") and .worker_id == $w)] | .[0].id // empty
' 2>/dev/null)
[ -z "$held" ] && exit 0
logs=$(planr --json log list --item "$held" 2>/dev/null | jq -r '
  [.logs[]? | select(.kind == "completion")] | length
' 2>/dev/null)
if [ "${logs:-0}" = "0" ]; then
  jq -nc --arg held "$held" '{followup_message: ("planr: " + $held + " is still picked with no completion log. Log evidence (planr done " + $held + " --summary ... --cmd ...) or release it (planr pick release " + $held + ").")}'
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
                // The guard file is planr-owned by name: refresh it
                // whenever the shipped content changed, so re-installs
                // upgrade old (e.g. unscoped) guard versions.
                let guard_path = self.root.join(GUARD_RELATIVE);
                let guard_current = fs::read_to_string(&guard_path).ok();
                if guard_current.as_deref() != Some(EVIDENCE_GUARD) {
                    crate::util::write_if_missing(&guard_path, EVIDENCE_GUARD, true)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&guard_path, fs::Permissions::from_mode(0o755))?;
                    }
                    result.written.push(GUARD_RELATIVE.to_string());
                }
                // sessionStart is the one Cursor event that injects
                // context from hook output; preCompact cannot restore
                // model context, so it is deliberately not wired.
                self.merge_hook_file(
                    ".cursor/hooks.json",
                    json!({"version": 1, "hooks": {}}),
                    Some("hooks"),
                    &[
                        (
                            "sessionStart",
                            "planr prime",
                            json!({"command": PRIME_COMMAND_CURSOR, "timeout": 10}),
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
                // SessionStart's `compact` source fires after
                // compaction, so one matcher group covers session start,
                // resume, and post-compaction refresh; PreCompact cannot
                // inject context and its envelope differs, so it is
                // deliberately not wired.
                self.merge_hook_file(
                    ".claude/settings.json",
                    json!({}),
                    Some("hooks"),
                    &[(
                        "SessionStart",
                        "planr prime",
                        json!({
                            "matcher": "startup|resume|compact",
                            "hooks": [{"type": "command", "command": PRIME_COMMAND_CLAUDE, "timeout": 10}],
                        }),
                    )],
                    &mut result,
                )?;
            }
            "codex" => {
                let stop_path = self.root.join(CODEX_STOP_RELATIVE);
                let stop_current = fs::read_to_string(&stop_path).ok();
                if stop_current.as_deref() != Some(CODEX_STOP_SCRIPT) {
                    crate::util::write_if_missing(&stop_path, CODEX_STOP_SCRIPT, true)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&stop_path, fs::Permissions::from_mode(0o755))?;
                    }
                    result.written.push(CODEX_STOP_RELATIVE.to_string());
                }
                // SessionStart covers fresh and post-compaction session
                // starts; PostCompact ignores stdout for context, so it
                // is deliberately not wired.
                self.merge_hook_file(
                    ".codex/hooks.json",
                    json!({"hooks": {}}),
                    Some("hooks"),
                    &[
                        (
                            "SessionStart",
                            "planr prime",
                            json!({"hooks": [{"type": "command", "command": PRIME_COMMAND_CODEX, "timeout": 10}]}),
                        ),
                        (
                            "Stop",
                            "planr-codex-stop",
                            json!({"hooks": [{"type": "command", "command": CODEX_STOP_RELATIVE, "timeout": 10}]}),
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
                        "{relative} exists but is not a JSON object planr can merge into; hooks skipped — add them manually (https://planr.so/docs/integrations)"
                    ));
                    return Ok(());
                }
            }
        } else {
            default
        };
        let mut changed = false;
        let events = match events_key {
            Some(key) => {
                if relative == ".codex/hooks.json"
                    && codex_hooks_have_unsupported_events(&root, key)
                    && !codex_unsupported_sidecar_mergeable(&path, result)?
                {
                    return Ok(());
                }
                let object = root.as_object_mut().expect("checked object above");
                let legacy_codex_events = if relative == ".codex/hooks.json" {
                    object
                        .iter()
                        .filter(|(name, value)| name.as_str() != key && value.is_array())
                        .map(|(name, _value)| name.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let mut moved_legacy_codex_events = Vec::new();
                for event in legacy_codex_events {
                    if codex_0145_hook_event_supported(&event) {
                        if let Some(value) = object.remove(&event) {
                            moved_legacy_codex_events.push((event, value));
                            changed = true;
                        }
                    } else if let Some(value) = object.remove(&event) {
                        preserve_unsupported_codex_event(&path, &event, value, result)?;
                        changed = true;
                    }
                }
                for (event, marker, _) in entries {
                    if let Some(list) = object.get_mut(*event).and_then(Value::as_array_mut) {
                        let before = list.len();
                        list.retain(|entry| !entry.to_string().contains(marker));
                        changed |= list.len() != before;
                        if list.is_empty() {
                            object.remove(*event);
                        }
                    }
                }
                let entry = object.entry(key.to_string()).or_insert_with(|| json!({}));
                match entry.as_object_mut() {
                    Some(events) => {
                        if relative == ".codex/hooks.json" {
                            let unsupported_nested = events
                                .iter()
                                .filter(|(name, _value)| !codex_0145_hook_event_supported(name))
                                .map(|(name, _value)| name.clone())
                                .collect::<Vec<_>>();
                            for event in unsupported_nested {
                                if let Some(value) = events.remove(&event) {
                                    preserve_unsupported_codex_event(&path, &event, value, result)?;
                                    changed = true;
                                }
                            }
                        }
                        for (event, value) in moved_legacy_codex_events {
                            let target = events.entry(event).or_insert_with(|| json!([]));
                            if let (Some(target), Some(source)) =
                                (target.as_array_mut(), value.as_array())
                            {
                                for hook in source {
                                    if !target.iter().any(|existing| existing == hook) {
                                        target.push(hook.clone());
                                    }
                                }
                            }
                        }
                        events
                    }
                    None => {
                        result.warnings.push(format!(
                            "{relative} has a non-object `{key}` section; hooks skipped — add them manually (https://planr.so/docs/integrations)"
                        ));
                        return Ok(());
                    }
                }
            }
            None => root.as_object_mut().expect("checked object above"),
        };
        // Retired events: earlier planr versions wired pre/post-compaction
        // events that cannot inject context. Planr-owned entries there are
        // removed on re-install; foreign entries stay.
        for retired in ["preCompact", "PreCompact", "PostCompact"] {
            if let Some(list) = events.get_mut(retired).and_then(Value::as_array_mut) {
                let before = list.len();
                list.retain(|entry| !entry.to_string().contains("planr prime"));
                changed |= list.len() != before;
                if list.is_empty() {
                    events.remove(retired);
                }
            }
        }
        for (event, marker, entry) in entries {
            changed |= reconcile_entry(events, event, marker, entry.clone());
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

fn preserve_unsupported_codex_event(
    hooks_path: &std::path::Path,
    event: &str,
    value: Value,
    result: &mut HookInstall,
) -> Result<()> {
    result.warnings.push(format!(
        ".codex/hooks.json contains unsupported Codex hook event {event}; preserved it in .codex/hooks.planr-unsupported.json and omitted it from active hooks"
    ));
    let unsupported_path = hooks_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("hooks.planr-unsupported.json");
    let mut preserved_root = if unsupported_path.exists() {
        let text = fs::read_to_string(&unsupported_path)?;
        match serde_json::from_str::<Value>(&text)? {
            Value::Object(object) => object,
            _ => {
                anyhow::bail!(
                    ".codex/hooks.planr-unsupported.json exists but is not a JSON object planr can merge into"
                );
            }
        }
    } else {
        serde_json::Map::new()
    };
    merge_unsupported_codex_value(&mut preserved_root, event, value);
    if let Some(parent) = unsupported_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        unsupported_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&Value::Object(preserved_root))?
        ),
    )?;
    if !result
        .written
        .iter()
        .any(|written| written == ".codex/hooks.planr-unsupported.json")
    {
        result
            .written
            .push(".codex/hooks.planr-unsupported.json".to_string());
    }
    Ok(())
}

fn codex_hooks_have_unsupported_events(root: &Value, key: &str) -> bool {
    let Some(object) = root.as_object() else {
        return false;
    };
    if object.iter().any(|(name, value)| {
        name.as_str() != key && value.is_array() && !codex_0145_hook_event_supported(name)
    }) {
        return true;
    }
    object
        .get(key)
        .and_then(Value::as_object)
        .is_some_and(|events| {
            events
                .keys()
                .any(|name| !codex_0145_hook_event_supported(name))
        })
}

fn codex_unsupported_sidecar_mergeable(
    hooks_path: &std::path::Path,
    result: &mut HookInstall,
) -> Result<bool> {
    let unsupported_path = hooks_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("hooks.planr-unsupported.json");
    if !unsupported_path.exists() {
        return Ok(true);
    }
    let text = fs::read_to_string(&unsupported_path)?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(_)) => Ok(true),
        Ok(_) => {
            result.warnings.push(
                ".codex/hooks.planr-unsupported.json exists but is not a JSON object planr can merge into; Codex unsupported hooks skipped to avoid data loss — fix or remove the sidecar and rerun install".to_string(),
            );
            Ok(false)
        }
        Err(error) => {
            result.warnings.push(format!(
                ".codex/hooks.planr-unsupported.json is not valid JSON ({error}); Codex unsupported hooks skipped to avoid data loss — fix or remove the sidecar and rerun install"
            ));
            Ok(false)
        }
    }
}

fn merge_unsupported_codex_value(
    preserved_root: &mut serde_json::Map<String, Value>,
    event: &str,
    value: Value,
) {
    match preserved_root.remove(event) {
        None => {
            preserved_root.insert(event.to_string(), value);
        }
        Some(existing) => {
            preserved_root.insert(event.to_string(), merge_json_values(existing, value));
        }
    }
}

fn merge_json_values(existing: Value, incoming: Value) -> Value {
    match (existing, incoming) {
        (Value::Array(mut existing), Value::Array(incoming)) => {
            for entry in incoming {
                if !existing.iter().any(|seen| seen == &entry) {
                    existing.push(entry);
                }
            }
            Value::Array(existing)
        }
        (Value::Array(mut existing), incoming) => {
            if !existing.iter().any(|seen| seen == &incoming) {
                existing.push(incoming);
            }
            Value::Array(existing)
        }
        (existing, Value::Array(mut incoming)) => {
            if !incoming.iter().any(|seen| seen == &existing) {
                incoming.insert(0, existing);
            }
            Value::Array(incoming)
        }
        (existing, incoming) if existing == incoming => existing,
        (existing, incoming) => Value::Array(vec![existing, incoming]),
    }
}

/// Reconciles the planr-owned entry for one event: appended when absent,
/// replaced in place when an owned entry (matched by `marker`) is
/// outdated, untouched when already current. Foreign entries are never
/// modified — that is the additive-merge contract; planr only owns what
/// carries its marker.
fn reconcile_entry(
    events: &mut serde_json::Map<String, Value>,
    event: &str,
    marker: &str,
    entry: Value,
) -> bool {
    let list = events.entry(event.to_string()).or_insert_with(|| json!([]));
    let Some(list) = list.as_array_mut() else {
        return false;
    };
    if let Some(existing) = list
        .iter_mut()
        .find(|existing| existing.to_string().contains(marker))
    {
        if *existing == entry {
            return false;
        }
        *existing = entry;
        return true;
    }
    list.push(entry);
    true
}
