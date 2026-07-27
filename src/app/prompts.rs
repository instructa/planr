use super::App;
use crate::cli::PromptCommand;
use crate::integrations::{install_snippet, mcp_json_config};
use anyhow::Result;
use serde_json::json;

impl App {
    pub(crate) fn prompt(&self, command: PromptCommand) -> Result<()> {
        let (mode, client) = match command {
            PromptCommand::Cli(args) => ("cli", args.client),
            PromptCommand::Mcp(args) => ("mcp", args.client),
            PromptCommand::Http(args) => ("http", args.client),
            PromptCommand::Routing(args) => return self.prompt_routing(args.client),
        };
        let client = client
            .map(|value| format!("{value:?}").to_lowercase())
            .unwrap_or_else(|| "generic".to_string());
        let prompt = match mode {
            "cli" => format!(
                "Use Planr as the local source of truth for planning and execution. Start with `planr project show --json`, inspect `planr map status --json`, pick work with `planr pick --json`, log evidence with `planr log add`, request and close reviews with `planr review ...`, and close only after `planr map preview --close <item-id>` is clean. Use database `{}` when an explicit DB path is needed. Target client: {client}.",
                self.db_path.display()
            ),
            "mcp" if client == "grok" => format!(
                "Configure Grok Build with this portable repository-local `.grok/config.toml` entry (no XAI credentials and no absolute database path):\n{}\nGrok hooks are unsupported/no-op in V1.",
                install_snippet("grok", &self.db_path)
            ),
            "mcp" => format!(
                "Configure a project-scoped MCP server with command `planr --db {} mcp`. Use `planr install codex|claude|cursor|grok --dry-run` for client-specific snippets, or this generic JSON:\n{}",
                self.db_path.display(),
                mcp_json_config(&self.db_path)
            ),
            "http" => "Run `planr serve --port 7526`, open `http://127.0.0.1:7526/review` for the local review workspace, use `/v1/review-workspace` for review data, `/v1/events/stream` for SSE, and keep the server bound to localhost.".to_string(),
            _ => unreachable!(),
        };
        self.emit(
            json!({
                "mode": mode,
                "client": client,
                "prompt": prompt,
                "global_config_edited": false
            }),
            prompt,
        )
    }
}
