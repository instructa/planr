use super::App;
use crate::cli::InstallClientArgs;
use crate::rolefiles::{install_artifact_paths, pi_workflow_assets};
use crate::util::write_if_missing;
use anyhow::Result;
use serde_json::json;

impl App {
    pub(super) fn install_pi(&self, args: InstallClientArgs) -> Result<()> {
        let artifact_paths = install_artifact_paths("pi", false, false);
        let mcp = json!({
            "supported": false,
            "status": "unsupported",
            "written": [],
        });
        let hooks = json!({
            "supported": false,
            "status": "unsupported",
            "written": [],
        });

        if args.dry_run {
            let mut human = String::from(
                "# Pi core uses repository-local Agent Skills and the Planr CLI; MCP and hooks are unsupported/no-op in V1.\n\
                 # `.pi/agents` roles activate only when optional pi-subagents is installed.\n\
                 # Non-dry install reconciles these repository artifacts:\n",
            );
            for path in &artifact_paths {
                human.push_str(&format!("{path}\n"));
            }
            return self.emit(
                json!({
                    "client": "pi",
                    "dry_run": true,
                    "artifacts": artifact_paths,
                    "mcp": mcp,
                    "hooks": hooks,
                    "global_config_edited": false,
                }),
                human,
            );
        }

        let mut assets = Vec::new();
        for (relative, content) in pi_workflow_assets() {
            let path = self.root.join(relative);
            write_if_missing(&path, content, args.force)?;
            assets.push(path);
        }
        self.emit(
            json!({
                "client": "pi",
                "assets": assets,
                "mcp": mcp,
                "hooks": hooks,
                "global_config_edited": false,
            }),
            "Pi repository-local skills and optional pi-subagents roles written; MCP and hooks unsupported/no-op in V1".to_string(),
        )
    }
}
