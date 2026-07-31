use serde::Serialize;
use serde_json::{Value, json};

mod cursor;
mod grok;
mod install;
mod mcp_tools;
pub use cursor::cursor_deeplink;
pub use grok::reconcile_grok_project_config;
pub use install::{install_snippet, mcp_json_config};
pub use mcp_tools::mcp_tools;

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
