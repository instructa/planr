use serde_json::Value;

pub const CODEX_0145_HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SessionStart",
    "SessionEnd",
    "SubagentStart",
    "SubagentStop",
    "UserPromptSubmit",
    "Stop",
];

pub const CODEX_0145_PERMISSION_MODES: &[&str] = &[
    "default",
    "acceptEdits",
    "plan",
    "dontAsk",
    "bypassPermissions",
];

pub fn codex_0145_hook_event_supported(event: &str) -> bool {
    CODEX_0145_HOOK_EVENTS.contains(&event)
}

pub fn validate_codex_0145_stop_input(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Stop input must be an object".to_string())?;
    let keys = [
        "session_id",
        "transcript_path",
        "cwd",
        "hook_event_name",
        "model",
        "permission_mode",
        "turn_id",
        "stop_hook_active",
        "last_assistant_message",
    ];
    if object.len() != keys.len() {
        return Err("Stop input must not contain additional properties".to_string());
    }
    for key in keys {
        if !object.contains_key(key) {
            return Err(format!("Stop input missing required field {key}"));
        }
    }
    require_string(value, "session_id")?;
    require_nullable_string(value, "transcript_path")?;
    require_string(value, "cwd")?;
    require_string(value, "model")?;
    require_string(value, "turn_id")?;
    if value["hook_event_name"].as_str() != Some("Stop") {
        return Err("Stop input hook_event_name must be Stop".to_string());
    }
    let permission_mode = value["permission_mode"]
        .as_str()
        .ok_or_else(|| "Stop input permission_mode must be a string".to_string())?;
    if !CODEX_0145_PERMISSION_MODES.contains(&permission_mode) {
        return Err(format!(
            "Stop input permission_mode {permission_mode} is not a Codex 0.145 value"
        ));
    }
    if !value["stop_hook_active"].is_boolean() {
        return Err("Stop input stop_hook_active must be boolean".to_string());
    }
    require_nullable_string(value, "last_assistant_message")?;
    Ok(())
}

pub fn validate_codex_0145_stop_output(value: &Value, expect_block: bool) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Stop output must be an object".to_string())?;
    if expect_block {
        if object.len() != 2 {
            return Err("block output must contain exactly decision and reason".to_string());
        }
        if value["decision"].as_str() != Some("block") {
            return Err("block output decision must be block".to_string());
        }
        require_nonempty_string(value, "reason", "Stop output")?;
    } else if !object.is_empty() {
        return Err("neutral allow output must be {}".to_string());
    }
    Ok(())
}

fn require_string(value: &Value, field: &'static str) -> Result<(), String> {
    value[field]
        .as_str()
        .map(|_| ())
        .ok_or_else(|| format!("Stop input {field} must be a string"))
}

fn require_nullable_string(value: &Value, field: &'static str) -> Result<(), String> {
    if value[field].is_null() || value[field].as_str().is_some() {
        return Ok(());
    }
    Err(format!("Stop input {field} must be a string or null"))
}

fn require_nonempty_string(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<(), String> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(|_| ())
        .ok_or_else(|| format!("{label} {field} must be a nonempty string"))
}

#[cfg(test)]
mod tests {
    use super::{CODEX_0145_PERMISSION_MODES, validate_codex_0145_stop_input};
    use serde_json::{Value, json};

    fn official_stop_schema_oracle(value: &Value) -> Result<(), String> {
        let object = value.as_object().ok_or_else(|| "root object".to_string())?;
        let required = [
            "session_id",
            "transcript_path",
            "cwd",
            "hook_event_name",
            "model",
            "permission_mode",
            "turn_id",
            "stop_hook_active",
            "last_assistant_message",
        ];
        if object.len() != required.len() {
            return Err("additional or missing properties".to_string());
        }
        for key in required {
            if !object.contains_key(key) {
                return Err(format!("missing {key}"));
            }
        }
        for key in ["session_id", "cwd", "model", "turn_id"] {
            if !value[key].is_string() {
                return Err(format!("{key} type"));
            }
        }
        for key in ["transcript_path", "last_assistant_message"] {
            if !(value[key].is_string() || value[key].is_null()) {
                return Err(format!("{key} type"));
            }
        }
        if value["hook_event_name"].as_str() != Some("Stop") {
            return Err("hook_event_name enum".to_string());
        }
        if !value["stop_hook_active"].is_boolean() {
            return Err("stop_hook_active type".to_string());
        }
        let permission_mode = value["permission_mode"]
            .as_str()
            .ok_or_else(|| "permission_mode type".to_string())?;
        if ![
            "default",
            "acceptEdits",
            "plan",
            "dontAsk",
            "bypassPermissions",
        ]
        .contains(&permission_mode)
        {
            return Err("permission_mode enum".to_string());
        }
        Ok(())
    }

    fn official_stop_fixture() -> Value {
        json!({
            "session_id": "",
            "transcript_path": null,
            "cwd": "",
            "hook_event_name": "Stop",
            "model": "",
            "permission_mode": "default",
            "turn_id": "",
            "stop_hook_active": false,
            "last_assistant_message": null
        })
    }

    #[test]
    fn codex_0145_stop_input_matches_independent_nullable_schema_oracle() {
        let mut value = official_stop_fixture();
        official_stop_schema_oracle(&value).unwrap();
        validate_codex_0145_stop_input(&value).unwrap();
        for mode in CODEX_0145_PERMISSION_MODES {
            value["permission_mode"] = json!(mode);
            official_stop_schema_oracle(&value).unwrap();
            validate_codex_0145_stop_input(&value).unwrap();
        }
        value["transcript_path"] = json!("/tmp/transcript.jsonl");
        value["last_assistant_message"] = json!("");
        official_stop_schema_oracle(&value).unwrap();
        validate_codex_0145_stop_input(&value).unwrap();
    }

    #[test]
    fn codex_0145_stop_input_rejects_missing_additional_type_and_old_permission() {
        let valid = official_stop_fixture();
        for (name, invalid) in [
            ("missing", {
                let mut value = valid.clone();
                value.as_object_mut().unwrap().remove("turn_id");
                value
            }),
            ("additional", {
                let mut value = valid.clone();
                value["extra"] = json!(true);
                value
            }),
            ("nullable_type", {
                let mut value = valid.clone();
                value["last_assistant_message"] = json!(false);
                value
            }),
            ("permission", {
                let mut value = valid.clone();
                value["permission_mode"] = json!("workspace-write");
                value
            }),
            ("hook_event", {
                let mut value = valid.clone();
                value["hook_event_name"] = json!("Notification");
                value
            }),
        ] {
            assert!(
                official_stop_schema_oracle(&invalid).is_err(),
                "{name} oracle should reject"
            );
            assert!(
                validate_codex_0145_stop_input(&invalid).is_err(),
                "{name} production validator should reject"
            );
        }
    }
}
