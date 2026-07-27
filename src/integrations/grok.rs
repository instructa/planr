use anyhow::{Result, anyhow};
use toml_edit::{DocumentMut, Item, Table};

/// Portable, project-scoped Grok Build MCP configuration.
fn project_config() -> String {
    r#"[mcp_servers.planr]
command = "planr"
args = ["mcp"]
enabled = true
startup_timeout_sec = 30
tool_timeout_sec = 6000
env = { PLANR_MCP_CLIENT = "grok" }
"#
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokConfigReconcile {
    pub content: String,
    pub changed: bool,
    pub conflict: bool,
}

/// Reconcile only Planr's table while retaining unrelated TOML and comments.
pub fn reconcile_grok_project_config(
    existing: Option<&str>,
    force: bool,
) -> Result<GrokConfigReconcile> {
    let Some(existing) = existing else {
        return Ok(GrokConfigReconcile {
            content: project_config(),
            changed: true,
            conflict: false,
        });
    };

    let semantic: toml::Value =
        toml::from_str(existing).map_err(|error| anyhow!("invalid .grok/config.toml: {error}"))?;
    let desired_semantic: toml::Value =
        toml::from_str(&project_config()).expect("static Grok project config must be valid TOML");
    let current_planr = semantic
        .get("mcp_servers")
        .and_then(|servers| servers.get("planr"));
    let desired_planr = desired_semantic
        .get("mcp_servers")
        .and_then(|servers| servers.get("planr"))
        .expect("static Grok config must contain mcp_servers.planr");
    if current_planr == Some(desired_planr) {
        return Ok(GrokConfigReconcile {
            content: existing.to_string(),
            changed: false,
            conflict: false,
        });
    }
    if current_planr.is_some() && !force {
        return Ok(GrokConfigReconcile {
            content: existing.to_string(),
            changed: false,
            conflict: true,
        });
    }

    let mut document = existing
        .parse::<DocumentMut>()
        .map_err(|error| anyhow!("invalid .grok/config.toml: {error}"))?;
    if !document.contains_key("mcp_servers") {
        document["mcp_servers"] = Item::Table(Table::new());
    }
    let servers_is_inline = document["mcp_servers"].is_inline_table();
    let servers = document["mcp_servers"]
        .as_table_like_mut()
        .ok_or_else(|| anyhow!("invalid .grok/config.toml: `mcp_servers` must be a table"))?;
    let existing_decor = servers.get("planr").and_then(|item| {
        item.as_table()
            .map(|table| table.decor().clone())
            .or_else(|| item.as_value().map(|value| value.decor().clone()))
    });
    let mut desired_document = project_config()
        .parse::<DocumentMut>()
        .expect("static Grok project config must be valid TOML");
    let mut desired_item = desired_document["mcp_servers"]
        .as_table_mut()
        .expect("static mcp_servers must be a table")
        .remove("planr")
        .expect("static mcp_servers.planr must exist");
    if let (Some(decor), Some(table)) = (existing_decor, desired_item.as_table_mut()) {
        *table.decor_mut() = decor;
    }
    if servers_is_inline {
        let desired_table = desired_item
            .into_table()
            .expect("static mcp_servers.planr must be a table");
        desired_item = Item::Value(toml_edit::Value::InlineTable(
            desired_table.into_inline_table(),
        ));
    }
    servers.insert("planr", desired_item);
    Ok(GrokConfigReconcile {
        content: document.to_string(),
        changed: true,
        conflict: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{project_config, reconcile_grok_project_config};

    #[test]
    fn renderer_round_trips_and_reconciliation_preserves_foreign_content() {
        let generated = project_config();
        let parsed: toml::Value = toml::from_str(&generated).unwrap();
        let planr = &parsed["mcp_servers"]["planr"];
        assert_eq!(planr["command"].as_str(), Some("planr"));
        assert_eq!(planr["args"][0].as_str(), Some("mcp"));
        assert_eq!(planr["env"]["PLANR_MCP_CLIENT"].as_str(), Some("grok"));
        for forbidden in ["XAI_API_KEY", "xai-", "--db", "auth", "model", "url"] {
            assert!(!generated.contains(forbidden));
        }
        let foreign = "# keep this heading\n[plugins]\npaths = [\"./tools\"] # keep this comment\n\n[mcp_servers.foreign]\ncommand = \"foreign-server\"\n";
        let merged = reconcile_grok_project_config(Some(foreign), false).unwrap();
        assert!(merged.changed && !merged.conflict);
        for retained in [
            "# keep this heading",
            "# keep this comment",
            "foreign-server",
        ] {
            assert!(merged.content.contains(retained));
        }
        let second = reconcile_grok_project_config(Some(&merged.content), false).unwrap();
        assert!(!second.changed && !second.conflict);
        assert_eq!(second.content, merged.content);
        let inline =
            "# inline stays inline\nmcp_servers = { foreign = { command = \"foreign\" } }\n";
        let inline_merged = reconcile_grok_project_config(Some(inline), false).unwrap();
        let inline_parsed: toml::Value = toml::from_str(&inline_merged.content).unwrap();
        assert!(inline_merged.content.contains("# inline stays inline"));
        assert_eq!(
            inline_parsed["mcp_servers"]["planr"]["command"].as_str(),
            Some("planr")
        );
    }

    #[test]
    fn requires_force_for_conflicts_and_never_rewrites_bad_toml() {
        let conflicting = "# foreign\n[mcp_servers.planr]\ncommand = \"custom-planr\"\nargs = [\"mcp\"]\n\n[permission]\ndefault = \"ask\"\n";
        let preserved = reconcile_grok_project_config(Some(conflicting), false).unwrap();
        assert!(preserved.conflict && !preserved.changed);
        assert_eq!(preserved.content, conflicting);
        let forced = reconcile_grok_project_config(Some(conflicting), true).unwrap();
        assert!(forced.changed && !forced.conflict);
        assert!(forced.content.contains("# foreign"));
        assert!(forced.content.contains("default = \"ask\""));
        assert!(!forced.content.contains("custom-planr"));
        assert!(reconcile_grok_project_config(Some("[broken\n"), true).is_err());
        assert!(
            reconcile_grok_project_config(Some("mcp_servers = \"not-a-table\"\n"), true).is_err()
        );
    }
}
