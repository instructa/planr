use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;

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
            "# Cursor project scope: .cursor/mcp.json\n# stdio is the V1 default. SSE/streamable HTTP can point at planr serve --port 8484 when enabled for local dashboard automation.\n{{\n  \"mcpServers\": {{\n    \"planr\": {{\"command\": \"planr\", \"args\": [\"--db\", \"{}\", \"mcp\"]}}\n  }}\n}}\n",
            db.display()
        ),
        _ => String::new(),
    }
}

pub fn mcp_json_config(db: &Path) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"planr\": {{\"command\": \"planr\", \"args\": [\"--db\", \"{}\", \"mcp\"]}}\n  }}\n}}\n",
        db.display()
    )
}

pub fn mcp_tools() -> Vec<Value> {
    let names = [
        ("planr_project_show", "Show current Planr project"),
        ("planr_map_show", "Show Planr map"),
        ("planr_map_status", "Show richer map status"),
        ("planr_map_preview", "Preview graph effects before mutation"),
        ("planr_map_unlocks", "Show what an item would unlock"),
        (
            "planr_map_lookahead",
            "Show near-term ready and blocked work",
        ),
        ("planr_plan_create", "Create a product plan package"),
        ("planr_plan_refine", "Append refinement context to a plan"),
        (
            "planr_plan_split",
            "Create a build plan from a product plan",
        ),
        ("planr_plan_check", "Validate a plan record and path"),
        ("planr_plan_link", "Link a plan source to an item"),
        ("planr_map_build", "Create map items from a plan"),
        ("planr_item_create", "Create a map item"),
        ("planr_item_breakdown", "Break an item into child items"),
        ("planr_item_insert", "Insert an item between linked work"),
        ("planr_item_amend", "Add future-work context to an item"),
        ("planr_item_replan", "Preview or replace pending child work"),
        ("planr_pick_item", "Atomically pick the next ready item"),
        (
            "planr_pick_heartbeat",
            "Record worker heartbeat for picked work",
        ),
        ("planr_pick_progress", "Record progress for picked work"),
        ("planr_pick_pause", "Pause picked work without releasing it"),
        ("planr_pick_resume", "Resume picked work"),
        ("planr_pick_stale", "Inspect stale picked work"),
        (
            "planr_recover_sweep",
            "Preview or apply stale, timed-out, and retryable recovery",
        ),
        (
            "planr_approval_request",
            "Request human approval before close",
        ),
        ("planr_approval_approve", "Approve item close gate"),
        ("planr_approval_deny", "Deny item close gate"),
        ("planr_approval_list", "List item approval gates"),
        (
            "planr_artifact_add",
            "Attach artifact metadata or small content",
        ),
        ("planr_artifact_list", "List artifacts"),
        ("planr_artifact_show", "Show artifact details"),
        ("planr_event_list", "List persisted events"),
        (
            "planr_debug_bundle",
            "Preview a privacy-minimized debug bundle",
        ),
        ("planr_log_add", "Add evidence log to an item"),
        ("planr_review_annotate", "Attach review annotation feedback"),
        (
            "planr_review_ingest",
            "Ingest hook-compatible review feedback",
        ),
        ("planr_review_artifact", "Write a review artifact"),
        (
            "planr_review_evidence",
            "Collect scoped Git and PR evidence for an item",
        ),
        ("planr_review_close", "Close a review item with verdict"),
        ("planr_close_item", "Close an item after checks"),
        ("planr_context_create", "Add project or item context"),
        ("planr_search", "Search items, plans, logs, and context"),
        ("planr_log_read", "Read one log entry"),
    ];
    names
        .into_iter()
        .map(|(name, description)| json!({"name": name, "description": description, "inputSchema": {"type": "object", "additionalProperties": true}}))
        .collect()
}

pub fn mcp_resources() -> Vec<Value> {
    vec![
        json!({"uri": "planr://project/map", "name": "Planr map", "mimeType": "application/json"}),
        json!({"uri": "planr://project/context", "name": "Planr project context", "mimeType": "application/json"}),
        json!({"uri": "planr://item/{id}", "name": "Planr item by id", "mimeType": "application/json"}),
        json!({"uri": "planr://plan/{id}", "name": "Planr plan by id", "mimeType": "application/json"}),
        json!({"uri": "planr://log/{id}", "name": "Planr log by id", "mimeType": "application/json"}),
    ]
}

pub fn mcp_json(value: impl Serialize) -> Value {
    json!({"content": [{"type": "text", "text": serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())}]})
}
