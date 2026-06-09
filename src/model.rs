use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub description: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub project_id: String,
    pub stage: String,
    pub path: String,
    pub title: String,
    pub slug: String,
    pub parse_status: String,
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub project_id: String,
    pub parent_item_id: Option<String>,
    pub title: String,
    pub description: String,
    pub status: String,
    pub work_type: String,
    pub priority: i64,
    pub worker_id: Option<String>,
    pub plan_path: Option<String>,
}
