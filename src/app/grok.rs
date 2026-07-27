use super::App;
use crate::cli::InstallClientArgs;
use crate::integrations::{install_snippet, reconcile_grok_project_config};
use crate::rolefiles::{grok_workflow_assets, install_artifact_paths};
use crate::util::write_if_missing;
use anyhow::{Result, bail};
use serde_json::json;
use std::fs;

impl App {
    pub(super) fn install_grok(&self, args: InstallClientArgs) -> Result<()> {
        let artifact_paths = install_artifact_paths("grok", !args.no_mcp, false);
        let hooks = json!({
            "supported": false,
            "status": "unsupported",
            "written": [],
        });
        if args.dry_run {
            let config = (!args.no_mcp).then(|| install_snippet("grok", &self.db_path));
            let mut human = String::new();
            if let Some(config) = &config {
                human.push_str(config);
            } else {
                human.push_str("# grok --no-mcp: repository-local roles and skills only.\n");
            }
            human.push_str("# Non-dry install reconciles these repository artifacts:\n");
            for path in &artifact_paths {
                human.push_str(&format!("{path}\n"));
            }
            human.push_str("# Grok hooks: unsupported/no-op in V1.");
            return self.emit(
                json!({
                    "client": "grok",
                    "dry_run": true,
                    "config": config,
                    "artifacts": artifact_paths,
                    "hooks": hooks,
                }),
                human,
            );
        }

        let mcp_path = if args.no_mcp {
            None
        } else {
            let path = self.root.join(".grok/config.toml");
            let existing = if path.exists() {
                Some(fs::read_to_string(&path)?)
            } else {
                None
            };
            let reconciled = reconcile_grok_project_config(existing.as_deref(), args.force)?;
            if reconciled.conflict {
                bail!(
                    "{} already defines mcp_servers.planr; re-run with --force to replace only that table",
                    path.display()
                );
            }
            if reconciled.changed {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, reconciled.content)?;
            }
            Some(path)
        };
        let mut assets = Vec::new();
        for (relative, content) in grok_workflow_assets() {
            let path = self.root.join(relative);
            write_if_missing(&path, content, args.force)?;
            assets.push(path);
        }
        self.emit(
            json!({
                "client": "grok",
                "path": mcp_path,
                "assets": assets,
                "hooks": hooks,
            }),
            if args.no_mcp {
                "grok repository-local roles and skills written (no MCP config); hooks unsupported/no-op in V1".to_string()
            } else {
                "grok integration written (portable project MCP config, repository-local roles and skills); hooks unsupported/no-op in V1".to_string()
            },
        )
    }
}
