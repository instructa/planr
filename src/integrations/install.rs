use std::path::Path;

use super::{cursor_deeplink, reconcile_grok_project_config};

pub fn install_snippet(client: &str, db: &Path) -> String {
    match client {
        "codex" => format!(
            "# Project-scoped Codex MCP snippet. Verify with: codex mcp list\n[mcp_servers.planr]\ncommand = \"planr\"\nargs = [\"--db\", \"{}\", \"mcp\"]\n",
            db.display()
        ),
        "claude" => format!(
            "# Project scope: write this repo's .mcp.json, or use the user-scope CLI command below when intentional.\n# User-scope command:\nclaude mcp add planr -- planr --db {} mcp\n{{\n  \"mcpServers\": {{\n    \"planr\": {{\"command\": \"planr\", \"args\": [\"--db\", \"{}\", \"mcp\"]}}\n  }}\n}}\n",
            db.display(),
            db.display()
        ),
        "cursor" => format!(
            "# Cursor project scope: .cursor/mcp.json\n# stdio is the V1 default. SSE/streamable HTTP can point at planr serve --port 7526 when enabled for local dashboard automation.\n{{\n  \"mcpServers\": {{\n    \"planr\": {{\"command\": \"planr\", \"args\": [\"--db\", \"{}\", \"mcp\"]}}\n  }}\n}}\n# One-click user-level MCP install (uses each workspace's own .planr db):\n{}\n# Non-dry install also writes .cursor/agents/planr-worker.md, .cursor/agents/planr-reviewer.md, and the Planr skills under .cursor/skills/.\n",
            db.display(),
            cursor_deeplink()
        ),
        "grok" => {
            reconcile_grok_project_config(None, false)
                .expect("static Grok project config must reconcile")
                .content
        }
        _ => String::new(),
    }
}

pub fn mcp_json_config(db: &Path) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"planr\": {{\"command\": \"planr\", \"args\": [\"--db\", \"{}\", \"mcp\"]}}\n  }}\n}}\n",
        db.display()
    )
}
