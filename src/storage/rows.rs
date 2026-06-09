use crate::model::{Item, Plan, Project};
use serde_json::{json, Value};

pub fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path: row.get(2)?,
        description: row.get(3)?,
        status: row.get(4)?,
    })
}

pub fn row_to_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<Plan> {
    let archived: i64 = row.get(7)?;
    Ok(Plan {
        id: row.get(0)?,
        project_id: row.get(1)?,
        stage: row.get(2)?,
        path: row.get(3)?,
        title: row.get(4)?,
        slug: row.get(5)?,
        parse_status: row.get(6)?,
        archived: archived != 0,
    })
}

pub fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_item_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: row.get(5)?,
        work_type: row.get(6)?,
        priority: row.get(7)?,
        worker_id: row.get(8)?,
        plan_path: row.get(9)?,
    })
}

pub fn row_to_log(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "item_id": row.get::<_, String>(1)?,
        "kind": row.get::<_, String>(2)?,
        "summary": row.get::<_, String>(3)?,
        "files": parse_json_cell(row.get::<_, Option<String>>(4)?),
        "commands": parse_json_cell(row.get::<_, Option<String>>(5)?),
        "tests": parse_json_cell(row.get::<_, Option<String>>(6)?),
        "review_findings": parse_json_cell(row.get::<_, Option<String>>(7)?),
        "created_at": row.get::<_, String>(8)?,
    }))
}

pub fn row_to_context(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "item_id": row.get::<_, Option<String>>(1)?,
        "kind": row.get::<_, String>(2)?,
        "content": row.get::<_, String>(3)?,
        "worker_id": row.get::<_, Option<String>>(4)?,
        "created_at": row.get::<_, String>(5)?,
    }))
}

fn parse_json_cell(text: Option<String>) -> Value {
    text.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}
