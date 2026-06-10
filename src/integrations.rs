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
            "# Cursor project scope: .cursor/mcp.json\n# stdio is the V1 default. SSE/streamable HTTP can point at planr serve --port 7526 when enabled for local dashboard automation.\n{{\n  \"mcpServers\": {{\n    \"planr\": {{\"command\": \"planr\", \"args\": [\"--db\", \"{}\", \"mcp\"]}}\n  }}\n}}\n",
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

fn prop(kind: &str, description: &str) -> Value {
    json!({"type": kind, "description": description})
}

fn string_array(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

pub fn mcp_tools() -> Vec<Value> {
    vec![
        tool(
            "planr_project_show",
            "Show current Planr project",
            json!({}),
            &[],
        ),
        tool("planr_map_show", "Show Planr map", json!({}), &[]),
        tool("planr_map_status", "Show richer map status", json!({}), &[]),
        tool(
            "planr_map_preview",
            "Preview graph effects before mutation",
            json!({"close": prop("string", "Item id whose close effect should be previewed")}),
            &["close"],
        ),
        tool(
            "planr_map_unlocks",
            "Show what an item would unlock",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_map_lookahead",
            "Show near-term ready and blocked work",
            json!({
                "from": prop("string", "Optional item id to start the lookahead from"),
                "limit": prop("integer", "Maximum entries to return (default 10)")
            }),
            &[],
        ),
        tool(
            "planr_plan_create",
            "Create a product plan package",
            json!({
                "title": prop("string", "Plan title"),
                "platform": prop("string", "Target platform label"),
                "ai": prop("boolean", "Include AI feature planning files"),
                "backend": prop("boolean", "Include backend planning files")
            }),
            &["title"],
        ),
        tool(
            "planr_plan_refine",
            "Append refinement context to a plan",
            json!({
                "id": prop("string", "Plan id"),
                "note": prop("string", "Refinement note to append")
            }),
            &["id"],
        ),
        tool(
            "planr_plan_split",
            "Create a build plan from a product plan",
            json!({
                "id": prop("string", "Source product plan id"),
                "slice": prop("string", "Build slice name")
            }),
            &["id", "slice"],
        ),
        tool(
            "planr_plan_check",
            "Validate a plan record and path",
            json!({"id": prop("string", "Plan id")}),
            &["id"],
        ),
        tool(
            "planr_plan_link",
            "Link a plan source to an item",
            json!({
                "source_id": prop("string", "Plan source id"),
                "item_id": prop("string", "Item id"),
                "relationship": prop("string", "Link relationship (default references)"),
                "section_id": prop("string", "Optional plan section id")
            }),
            &["source_id", "item_id"],
        ),
        tool(
            "planr_map_build",
            "Create map items from a plan",
            json!({"from": prop("string", "Plan id to build items from")}),
            &["from"],
        ),
        tool(
            "planr_item_create",
            "Create a map item",
            json!({
                "title": prop("string", "Item title"),
                "description": prop("string", "Item description"),
                "work_type": prop("string", "Work type (default generic)"),
                "after": prop("string", "Existing item id this item depends on"),
                "timeout_seconds": prop("integer", "Runtime timeout before the pick is stale"),
                "max_retries": prop("integer", "Maximum automatic retries"),
                "retry_delay_ms": prop("integer", "Base retry delay in milliseconds"),
                "retry_backoff": prop("string", "Retry backoff strategy"),
                "pre": prop("string", "Pre-condition note"),
                "post": prop("string", "Post-condition note")
            }),
            &["title", "description"],
        ),
        tool(
            "planr_item_breakdown",
            "Break an item into child items",
            json!({
                "id": prop("string", "Parent item id"),
                "into": prop("string", "Comma-separated child titles")
            }),
            &["id", "into"],
        ),
        tool(
            "planr_item_insert",
            "Insert an item between linked work",
            json!({
                "title": prop("string", "New item title"),
                "description": prop("string", "New item description"),
                "after": prop("string", "Item id the new item comes after"),
                "before": prop("string", "Optional item id the new item comes before"),
                "confirm": prop("boolean", "Apply the insert instead of previewing")
            }),
            &["title", "description", "after"],
        ),
        tool(
            "planr_item_amend",
            "Add future-work context to an item",
            json!({
                "id": prop("string", "Item id"),
                "note": prop("string", "Amendment content"),
                "tag": prop("string", "Context kind label (default amendment)")
            }),
            &["id", "note"],
        ),
        tool(
            "planr_item_replan",
            "Preview or replace pending child work",
            json!({
                "parent_id": prop("string", "Parent item id"),
                "into": prop("string", "Comma-separated replacement child titles"),
                "confirm": prop("boolean", "Apply the replan instead of previewing")
            }),
            &["parent_id", "into"],
        ),
        tool(
            "planr_pick_item",
            "Atomically pick the next ready item",
            json!({}),
            &[],
        ),
        tool(
            "planr_pick_heartbeat",
            "Record worker heartbeat for picked work",
            json!({"item_id": prop("string", "Item id (defaults to this worker's current pick)")}),
            &[],
        ),
        tool(
            "planr_pick_progress",
            "Record progress for picked work",
            json!({
                "item_id": prop("string", "Item id"),
                "percent": prop("integer", "Progress percent 0-100"),
                "note": prop("string", "Optional progress note")
            }),
            &["item_id"],
        ),
        tool(
            "planr_pick_pause",
            "Pause picked work without releasing it",
            json!({
                "item_id": prop("string", "Item id"),
                "note": prop("string", "Optional pause note")
            }),
            &["item_id"],
        ),
        tool(
            "planr_pick_resume",
            "Resume picked work",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_pick_stale",
            "Inspect stale picked work",
            json!({"older_than_seconds": prop("integer", "Staleness threshold in seconds (default 900)")}),
            &[],
        ),
        tool(
            "planr_recover_sweep",
            "Preview or apply stale, timed-out, and retryable recovery",
            json!({
                "older_than_seconds": prop("integer", "Staleness threshold in seconds (default 900)"),
                "apply": prop("boolean", "Apply recovery instead of previewing")
            }),
            &[],
        ),
        tool(
            "planr_approval_request",
            "Request human approval before close",
            json!({
                "item_id": prop("string", "Item id"),
                "reason": prop("string", "Why approval is needed")
            }),
            &["item_id"],
        ),
        tool(
            "planr_approval_approve",
            "Approve item close gate",
            json!({
                "item_id": prop("string", "Item id"),
                "by": prop("string", "Approver identity"),
                "comment": prop("string", "Optional approval comment")
            }),
            &["item_id", "by"],
        ),
        tool(
            "planr_approval_deny",
            "Deny item close gate",
            json!({
                "item_id": prop("string", "Item id"),
                "by": prop("string", "Denier identity"),
                "comment": prop("string", "Optional denial comment")
            }),
            &["item_id", "by"],
        ),
        tool(
            "planr_approval_list",
            "List item approval gates",
            json!({"open": prop("boolean", "Only list open approval requests")}),
            &[],
        ),
        tool(
            "planr_artifact_add",
            "Attach artifact metadata or small content",
            json!({
                "name": prop("string", "Artifact name"),
                "item": prop("string", "Optional item id"),
                "kind": prop("string", "Artifact kind (default evidence)"),
                "path": prop("string", "Optional file path reference"),
                "content": prop("string", "Optional inline content"),
                "mime": prop("string", "MIME type (default text/plain)")
            }),
            &["name"],
        ),
        tool(
            "planr_artifact_list",
            "List artifacts",
            json!({"item": prop("string", "Optional item id filter")}),
            &[],
        ),
        tool(
            "planr_artifact_show",
            "Show artifact details",
            json!({"id": prop("string", "Artifact id")}),
            &["id"],
        ),
        tool(
            "planr_event_list",
            "List persisted events",
            json!({
                "item": prop("string", "Optional item id filter"),
                "limit": prop("integer", "Maximum events (default 50)")
            }),
            &[],
        ),
        tool(
            "planr_debug_bundle",
            "Preview a privacy-minimized debug bundle",
            json!({"item": prop("string", "Optional item id filter")}),
            &[],
        ),
        tool(
            "planr_log_add",
            "Add evidence log to an item",
            json!({
                "item": prop("string", "Item id"),
                "summary": prop("string", "What was done"),
                "kind": prop("string", "Log kind (default completion)"),
                "files": string_array("Changed file paths"),
                "commands": string_array("Commands run"),
                "tests": string_array("Tests run with results")
            }),
            &["item", "summary"],
        ),
        tool(
            "planr_review_annotate",
            "Attach review annotation feedback",
            json!({
                "item_id": prop("string", "Item id"),
                "message": prop("string", "Annotation message"),
                "severity": prop("string", "Severity (default info)"),
                "author": prop("string", "Annotation author"),
                "file": prop("string", "File path the annotation refers to"),
                "line": prop("integer", "Line number the annotation refers to")
            }),
            &["item_id", "message"],
        ),
        // review_ingest intentionally accepts arbitrary hook payload shapes.
        json!({
            "name": "planr_review_ingest",
            "description": "Ingest hook-compatible review feedback",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "item_id": prop("string", "Item id"),
                    "feedback": {"type": "object", "description": "Review feedback payload"},
                    "payload": {"type": "object", "description": "Alternative feedback payload key"}
                },
                "required": ["item_id"],
                "additionalProperties": true
            }
        }),
        tool(
            "planr_review_artifact",
            "Write a review artifact",
            json!({"review_item_id": prop("string", "Review item id")}),
            &["review_item_id"],
        ),
        tool(
            "planr_review_evidence",
            "Collect scoped Git and PR evidence for an item",
            json!({
                "item_id": prop("string", "Item id"),
                "pr_url": prop("string", "Optional pull request URL to record")
            }),
            &["item_id"],
        ),
        tool(
            "planr_review_close",
            "Close a review item with verdict",
            json!({
                "review_item_id": prop("string", "Review item id"),
                "verdict": prop("string", "Verdict: complete, partial, failed, or unclear"),
                "findings": string_array("Findings discovered during review")
            }),
            &["review_item_id"],
        ),
        tool(
            "planr_close_item",
            "Close an item after checks",
            json!({"item_id": prop("string", "Item id")}),
            &["item_id"],
        ),
        tool(
            "planr_context_create",
            "Add project or item context",
            json!({
                "content": prop("string", "Context content"),
                "item": prop("string", "Optional item id"),
                "kind": prop("string", "Context kind (default discovery)")
            }),
            &["content"],
        ),
        tool(
            "planr_search",
            "Search items, plans, logs, and context",
            json!({"query": prop("string", "Search query")}),
            &["query"],
        ),
        tool(
            "planr_log_read",
            "Read one log entry",
            json!({"id": prop("string", "Log id")}),
            &["id"],
        ),
    ]
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
