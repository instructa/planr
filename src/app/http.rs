use super::{recovery::ItemRecoveryInput, App, ReviewAnnotationInput};
use crate::cli::ServeArgs;
use crate::util::{infer_error_code, item_id, path_item_id, short_id, worker_id};
use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};

impl App {
    pub(crate) fn serve(&self, args: ServeArgs) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", args.port))?;
        eprintln!("planr serve listening on http://127.0.0.1:{}", args.port);
        for stream in listener.incoming() {
            // A single misbehaving connection must never terminate the server.
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("planr serve: accept error: {error}");
                    continue;
                }
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
            // One thread and one SQLite connection per request keeps slow
            // requests and held SSE streams from blocking the server.
            let root = self.root.clone();
            let db_path = self.db_path.clone();
            let json = self.json;
            std::thread::spawn(move || {
                let app = match crate::storage::open_db(&db_path) {
                    Ok(conn) => App {
                        conn,
                        root,
                        db_path,
                        json,
                    },
                    Err(error) => {
                        eprintln!("planr serve: database open error: {error:#}");
                        return;
                    }
                };
                if let Err(error) = app.handle_http(stream) {
                    eprintln!("planr serve: connection error: {error:#}");
                }
            });
        }
        Ok(())
    }

    pub(crate) fn handle_http(&self, mut stream: TcpStream) -> Result<()> {
        let mut reader = io::BufReader::new(stream.try_clone()?);
        let mut first = String::new();
        reader.read_line(&mut first)?;
        let method = first.split_whitespace().next().unwrap_or("GET");
        let raw_path = first.split_whitespace().nth(1).unwrap_or("/");
        let path = raw_path.split('?').next().unwrap_or(raw_path);
        let query = raw_path.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut content_length = 0usize;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header)?;
            let trimmed = header.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
        if content_length > MAX_BODY_BYTES {
            let body = serde_json::to_string(&json!({
                "error": {"code": "payload_too_large", "message": "request body exceeds 10 MiB limit", "details": {}}
            }))?;
            write!(
                stream,
                "HTTP/1.1 413 Payload Too Large\r\n{CORS_HEADERS}content-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            )?;
            return Ok(());
        }
        let mut raw_body = vec![0u8; content_length];
        if content_length > 0 {
            reader.read_exact(&mut raw_body)?;
        }
        if method == "OPTIONS" {
            write!(
                stream,
                "HTTP/1.1 204 No Content\r\n{CORS_HEADERS}content-length: 0\r\n\r\n"
            )?;
            return Ok(());
        }
        if method == "GET" && path == "/v1/events/stream" {
            return self.stream_events(stream);
        }
        let body_json: Value = if raw_body.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice(&raw_body) {
                Ok(value) => value,
                Err(error) => {
                    let body = serde_json::to_string(&json!({
                        "error": {"code": "bad_request", "message": format!("request body is not valid JSON: {error}"), "details": {}}
                    }))?;
                    write!(
                        stream,
                        "HTTP/1.1 400 Bad Request\r\n{CORS_HEADERS}content-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    )?;
                    return Ok(());
                }
            }
        };
        let body_result: Result<String> = (|| {
            let body = match (method, path) {
                ("GET", "/review") | ("GET", "/review/") => self.review_workspace_html(),
                ("GET", "/v1/review-workspace") => {
                    serde_json::to_string(&self.review_workspace_value()?)?
                }
                ("GET", "/v1/projects") => {
                    serde_json::to_string(&json!({"projects": self.list_projects()?}))?
                }
                ("POST", "/v1/projects") => {
                    let name = body_json
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Planr Project");
                    let id = short_id("p");
                    self.conn.execute(
                    "INSERT INTO projects(id, name, root_path, description, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'active', datetime('now'), datetime('now'))",
                    params![id, name, self.root.to_string_lossy(), body_json.get("description").and_then(Value::as_str).unwrap_or("Planr project")],
                )?;
                    serde_json::to_string(&json!({"project": self.get_project(&id)?}))?
                }
                ("GET", p) if p.ends_with("/map") => serde_json::to_string(&self.map_value()?)?,
                ("GET", p) if p.ends_with("/map/status") => {
                    serde_json::to_string(&self.map_status_value()?)?
                }
                ("GET", p) if p.ends_with("/map/lookahead") => {
                    let from = query
                        .split('&')
                        .filter_map(|part| part.split_once('='))
                        .find(|(key, _)| *key == "from")
                        .map(|(_, value)| crate::util::url_decode(value));
                    serde_json::to_string(&self.lookahead_value(from.as_deref(), 10)?)?
                }
                ("GET", p) if p.ends_with("/items") => {
                    serde_json::to_string(&json!({"items": self.all_items()?}))?
                }
                ("POST", p) if p.ends_with("/items") => {
                    let item = self.create_item(
                        None,
                        body_json
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("Untitled item"),
                        body_json
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                        body_json
                            .get("work_type")
                            .and_then(Value::as_str)
                            .unwrap_or("generic"),
                        body_json.get("plan_path").and_then(Value::as_str),
                    )?;
                    if body_json.get("timeout_seconds").is_some()
                        || body_json.get("max_retries").is_some()
                        || body_json.get("retry_delay_ms").is_some()
                        || body_json.get("retry_backoff").is_some()
                        || body_json.get("pre").is_some()
                        || body_json.get("post").is_some()
                    {
                        self.configure_item_recovery(
                            &item.id,
                            ItemRecoveryInput {
                                timeout_seconds: body_json
                                    .get("timeout_seconds")
                                    .and_then(Value::as_i64),
                                max_retries: body_json.get("max_retries").and_then(Value::as_i64),
                                retry_backoff: body_json
                                    .get("retry_backoff")
                                    .and_then(Value::as_str),
                                retry_delay_ms: body_json
                                    .get("retry_delay_ms")
                                    .and_then(Value::as_i64),
                                pre_condition: body_json.get("pre").and_then(Value::as_str),
                                post_condition: body_json.get("post").and_then(Value::as_str),
                            },
                        )?;
                    }
                    serde_json::to_string(&json!({"item": item}))?
                }
                ("GET", p) if p.ends_with("/unlocks") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in unlocks route"))?;
                    serde_json::to_string(
                        &json!({"item_id": item_id, "would_unlock": self.would_unlock_items(item_id)?}),
                    )?
                }
                ("GET", p) if p.ends_with("/preview-close") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in preview-close route"))?;
                    serde_json::to_string(&self.preview_close_value(item_id)?)?
                }
                ("POST", "/v1/recover/sweep") => {
                    let older_than_seconds = body_json
                        .get("older_than_seconds")
                        .and_then(Value::as_i64)
                        .unwrap_or(900);
                    let apply = body_json
                        .get("apply")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    serde_json::to_string(&self.recovery_sweep_value(older_than_seconds, apply)?)?
                }
                ("POST", p) if p.ends_with("/insert") => {
                    let after = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in insert route"))?;
                    let title = body_json
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Inserted item")
                        .to_string();
                    let description = body_json
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let before = body_json.get("before").and_then(Value::as_str);
                    let confirm = body_json
                        .get("confirm")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if !confirm {
                        serde_json::to_string(&json!({
                            "mode": "preview",
                            "action": "insert",
                            "would_create": {"title": title, "description": description, "after": after, "before": before},
                        }))?
                    } else {
                        let project = self.default_project()?;
                        let id = item_id(&title);
                        let tx = self.conn.unchecked_transaction()?;
                        tx.execute(
                        "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, created_at, updated_at) VALUES (?1, ?2, NULL, ?3, ?4, 'pending', 'generic', 0, datetime('now'), datetime('now'))",
                        params![id, project.id, title, description],
                    )?;
                        if let Some(before) = before {
                            tx.execute(
                            "DELETE FROM links WHERE from_item = ?1 AND to_item = ?2 AND kind = 'blocks'",
                            params![after, before],
                        )?;
                        }
                        tx.execute(
                        "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                        params![after, id],
                    )?;
                        if let Some(before) = before {
                            tx.execute(
                            "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                            params![id, before],
                        )?;
                        }
                        tx.execute(
                        "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES ('item', ?1, ?2, ?3, NULL)",
                        params![id, title, description],
                    )?;
                        tx.commit()?;
                        if let Some(before) = before {
                            self.demote_if_blocked(before)?;
                        }
                        self.demote_if_blocked(&id)?;
                        self.promote_ready()?;
                        serde_json::to_string(&json!({"item": self.get_item(&id)?}))?
                    }
                }
                ("POST", p) if p.ends_with("/amend") => {
                    let item_id =
                        path_item_id(p).ok_or_else(|| anyhow!("missing item id in amend route"))?;
                    let item = self.get_item(item_id)?;
                    if matches!(
                        item.status.as_str(),
                        "closed" | "closed_partial" | "cancelled"
                    ) {
                        bail!("cannot amend item {} from status {}", item.id, item.status);
                    }
                    let id = short_id("ctx");
                    let note = body_json.get("note").and_then(Value::as_str).unwrap_or("");
                    let tag = body_json
                        .get("tag")
                        .and_then(Value::as_str)
                        .unwrap_or("amendment");
                    self.conn.execute(
                    "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
                    params![id, self.default_project()?.id, item.id, worker_id(), tag, note, json!(["amend"]).to_string()],
                )?;
                    self.index_search("context", &id, tag, note, None)?;
                    serde_json::to_string(
                        &json!({"item": item, "context": self.get_context(&id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/replan") => {
                    let parent_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in replan route"))?;
                    let parent = self.get_item(parent_id)?;
                    let into = body_json.get("into").and_then(Value::as_str).unwrap_or("");
                    let titles = into
                        .split(',')
                        .map(str::trim)
                        .filter(|title| !title.is_empty())
                        .collect::<Vec<_>>();
                    let cancellable =
                        self.child_items_by_statuses(parent_id, &["pending", "ready", "blocked"])?;
                    let confirm = body_json
                        .get("confirm")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if !confirm {
                        serde_json::to_string(
                            &json!({"mode": "preview", "action": "replan", "parent": parent, "would_cancel": cancellable, "would_create": titles}),
                        )?
                    } else {
                        if !self
                            .child_items_by_statuses(
                                parent_id,
                                &["picked", "running", "in_review"],
                            )?
                            .is_empty()
                        {
                            bail!(
                                "cannot replan while child items are picked, running, or in review"
                            );
                        }
                        let project = self.default_project()?;
                        let tx = self.conn.unchecked_transaction()?;
                        for child in &cancellable {
                            tx.execute(
                                "DELETE FROM links WHERE from_item = ?1 OR to_item = ?1",
                                params![child.id],
                            )?;
                        }
                        tx.execute(
                        "UPDATE items SET status = 'cancelled', updated_at = datetime('now') WHERE parent_item_id = ?1 AND status IN ('pending','ready','blocked')",
                        params![parent_id],
                    )?;
                        let mut previous: Option<String> = None;
                        let mut created_ids = Vec::new();
                        for title in titles {
                            let id = item_id(title);
                            tx.execute(
                            "INSERT INTO items(id, project_id, parent_item_id, title, description, status, work_type, priority, plan_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 'generic', 0, ?6, datetime('now'), datetime('now'))",
                            params![id, project.id, parent_id, title, format!("Replanned child for {}", parent.title), parent.plan_path.as_deref()],
                        )?;
                            tx.execute(
                            "INSERT INTO search_index(source_type, source_id, title, body, path) VALUES ('item', ?1, ?2, ?3, ?4)",
                            params![id, title, format!("Replanned child for {}", parent.title), parent.plan_path.as_deref()],
                        )?;
                            if let Some(prev) = previous {
                                tx.execute(
                                "INSERT OR IGNORE INTO links(from_item, to_item, kind, condition) VALUES (?1, ?2, 'blocks', 'all')",
                                params![prev, id],
                            )?;
                            }
                            previous = Some(id.clone());
                            created_ids.push(id);
                        }
                        tx.commit()?;
                        self.promote_ready()?;
                        let created = created_ids
                            .iter()
                            .map(|id| self.get_item(id))
                            .collect::<Result<Vec<_>>>()?;
                        serde_json::to_string(
                            &json!({"cancelled": cancellable, "created": created}),
                        )?
                    }
                }
                ("POST", p) if p.ends_with("/heartbeat") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in heartbeat route"))?;
                    self.heartbeat_item(item_id)?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/progress") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in progress route"))?;
                    let percent = body_json
                        .get("percent")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    if !(0..=100).contains(&percent) {
                        bail!("progress percent must be between 0 and 100");
                    }
                    self.progress_item(
                        item_id,
                        percent,
                        body_json.get("note").and_then(Value::as_str),
                    )?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/pause") => {
                    let item_id =
                        path_item_id(p).ok_or_else(|| anyhow!("missing item id in pause route"))?;
                    self.pause_item(item_id, body_json.get("note").and_then(Value::as_str))?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/resume") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in resume route"))?;
                    self.resume_item(item_id)?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "runtime": self.item_runtime(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/approval/request") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in approval request route"))?;
                    self.conn.execute(
                    "UPDATE items SET approval_status = 'requested', approval_requested_at = datetime('now'), approval_comment = ?1, approved_by = NULL, updated_at = datetime('now') WHERE id = ?2",
                    params![body_json.get("reason").and_then(Value::as_str), item_id],
                )?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "approval": self.item_approval(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/approval/approve") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in approval approve route"))?;
                    let by = body_json
                        .get("by")
                        .and_then(Value::as_str)
                        .unwrap_or("human");
                    self.conn.execute(
                    "UPDATE items SET approval_status = 'approved', approved_by = ?1, approval_comment = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![by, body_json.get("comment").and_then(Value::as_str), item_id],
                )?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "approval": self.item_approval(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/approval/deny") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in approval deny route"))?;
                    let by = body_json
                        .get("by")
                        .and_then(Value::as_str)
                        .unwrap_or("human");
                    self.conn.execute(
                    "UPDATE items SET approval_status = 'denied', approved_by = ?1, approval_comment = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![by, body_json.get("comment").and_then(Value::as_str), item_id],
                )?;
                    serde_json::to_string(
                        &json!({"item": self.get_item(item_id)?, "approval": self.item_approval(item_id)?}),
                    )?
                }
                ("GET", "/v1/approvals") => {
                    let open = query
                        .split('&')
                        .filter_map(|part| part.split_once('='))
                        .any(|(key, value)| key == "open" && value == "true");
                    serde_json::to_string(&json!({"approvals": self.list_approvals(open)?}))?
                }
                ("POST", "/v1/pick") => serde_json::to_string(&self.next_pick_value(
                    None,
                    body_json.get("work_type").and_then(Value::as_str),
                    body_json.get("plan").and_then(Value::as_str),
                )?)?,
                ("POST", p) if p.ends_with("/log") => {
                    let item_id =
                        path_item_id(p).ok_or_else(|| anyhow!("missing item id in log route"))?;
                    let id = short_id("log");
                    let summary = body_json
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("HTTP log");
                    let commands = body_json
                        .get("commands")
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                    let run_id = commands
                        .as_array()
                        .filter(|a| !a.is_empty())
                        .map(|values| {
                            let commands = values
                                .iter()
                                .filter_map(Value::as_str)
                                .map(ToOwned::to_owned)
                                .collect::<Vec<_>>();
                            self.record_run(item_id, &commands, "closed")
                        })
                        .transpose()?;
                    self.conn.execute(
                    "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, files, commands, tests, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
                    params![
                        id,
                        self.default_project()?.id,
                        item_id,
                        run_id,
                        body_json.get("kind").and_then(Value::as_str).unwrap_or("completion"),
                        summary,
                        serde_json::to_string(body_json.get("files").unwrap_or(&json!([])))?,
                        serde_json::to_string(&commands)?,
                        serde_json::to_string(body_json.get("tests").unwrap_or(&json!([])))?,
                    ],
                )?;
                    self.index_search("log", &id, summary, summary, None)?;
                    self.record_event(
                        "log_created",
                        Some(item_id),
                        json!({"log_id": id.clone(), "kind": body_json.get("kind").and_then(Value::as_str).unwrap_or("completion"), "source": "http"}),
                    )?;
                    serde_json::to_string(&json!({"log": self.get_log(&id)?}))?
                }
                ("POST", p) if p.starts_with("/v1/reviews/") && p.ends_with("/close") => {
                    let review_id = p
                        .trim_start_matches("/v1/reviews/")
                        .trim_end_matches("/close")
                        .trim_end_matches('/');
                    let verdict = body_json
                        .get("verdict")
                        .and_then(Value::as_str)
                        .unwrap_or("unclear");
                    let findings = body_json
                        .get("findings")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(ToOwned::to_owned)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    serde_json::to_string(
                        &self.close_review_item(
                            review_id,
                            verdict,
                            findings,
                            "http",
                            body_json.get("reviewer").and_then(Value::as_str),
                            body_json
                                .get("close_target")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        )?,
                    )?
                }
                ("GET", p) if p.starts_with("/v1/reviews/") && p.ends_with("/artifact") => {
                    let review_id = p
                        .trim_start_matches("/v1/reviews/")
                        .trim_end_matches("/artifact")
                        .trim_end_matches('/');
                    serde_json::to_string(&json!({"artifact": self.latest_review_artifact(
                        review_id,
                    )?}))?
                }
                ("POST", p) if p.starts_with("/v1/reviews/") && p.ends_with("/artifact") => {
                    let review_id = p
                        .trim_start_matches("/v1/reviews/")
                        .trim_end_matches("/artifact")
                        .trim_end_matches('/');
                    serde_json::to_string(&json!({"artifact": self.write_review_artifact(
                        review_id,
                        None,
                        &[],
                        &[],
                        None,
                        None,
                    )?}))?
                }
                ("POST", p) if p.ends_with("/close") => {
                    let item_id =
                        path_item_id(p).ok_or_else(|| anyhow!("missing item id in close route"))?;
                    self.promote_ready()?;
                    self.ensure_can_close(item_id)?;
                    self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item_id])?;
                    self.promote_ready()?;
                    self.record_event("item_closed", Some(item_id), json!({"source": "http"}))?;
                    serde_json::to_string(&json!({"closed": item_id, "map": self.map_value()?}))?
                }
                ("POST", p) if p.ends_with("/reviews") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in reviews route"))?;
                    let review = self.request_review_for(item_id)?;
                    serde_json::to_string(&json!({"review": review}))?
                }
                ("POST", p) if p.ends_with("/review-annotations") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in review annotation route"))?;
                    let message = body_json
                        .get("message")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("missing review annotation message"))?;
                    serde_json::to_string(&json!({"annotation": self.add_review_annotation(
                        ReviewAnnotationInput {
                            item_id,
                            message,
                            severity: body_json.get("severity").and_then(Value::as_str).unwrap_or("info"),
                            author: body_json.get("author").and_then(Value::as_str),
                            file: body_json.get("file").and_then(Value::as_str),
                            line: body_json.get("line").and_then(Value::as_u64),
                            source: "http",
                        }
                    )?}))?
                }
                ("GET", p) if p.ends_with("/review-evidence") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in evidence route"))?;
                    serde_json::to_string(
                        &json!({"evidence": self.review_evidence_value(item_id)?}),
                    )?
                }
                ("POST", p) if p.ends_with("/review-evidence") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in evidence route"))?;
                    let pr_context = body_json
                        .get("pr_url")
                        .and_then(Value::as_str)
                        .map(|url| self.record_pr_url(item_id, url))
                        .transpose()?;
                    serde_json::to_string(&json!({
                        "evidence": self.review_evidence_value(item_id)?,
                        "pr_context": pr_context
                    }))?
                }
                ("POST", p) if p.ends_with("/review-feedback") => {
                    let item_id = path_item_id(p)
                        .ok_or_else(|| anyhow!("missing item id in review feedback route"))?;
                    serde_json::to_string(&self.ingest_review_feedback(
                        item_id,
                        body_json.clone(),
                        "http",
                    )?)?
                }
                ("POST", "/v1/contexts") => {
                    let id = short_id("ctx");
                    let content = body_json
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let kind = body_json
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("discovery");
                    self.conn.execute(
                    "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', datetime('now'))",
                    params![id, self.default_project()?.id, body_json.get("item").and_then(Value::as_str), worker_id(), kind, content],
                )?;
                    self.index_search("context", &id, kind, content, None)?;
                    self.record_event(
                        "context_created",
                        body_json.get("item").and_then(Value::as_str),
                        json!({"context_id": id.clone(), "kind": kind, "source": "http"}),
                    )?;
                    serde_json::to_string(&json!({"context": self.get_context(&id)?}))?
                }
                ("POST", "/v1/artifacts") => {
                    let name = body_json
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("artifact");
                    let item = body_json.get("item").and_then(Value::as_str);
                    if let Some(item_id) = item {
                        self.get_item(item_id)?;
                    }
                    let id = short_id("art");
                    let content = body_json.get("content").and_then(Value::as_str);
                    self.conn.execute(
                        "INSERT INTO artifacts(id, project_id, item_id, name, kind, path, content, mime_type, size_bytes, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
                        params![
                            id,
                            self.default_project()?.id,
                            item,
                            name,
                            body_json.get("kind").and_then(Value::as_str).unwrap_or("evidence"),
                            body_json.get("path").and_then(Value::as_str),
                            content,
                            body_json.get("mime").and_then(Value::as_str).unwrap_or("text/plain"),
                            content.map(|value| value.len() as i64),
                            json!({"source": "http"}).to_string(),
                        ],
                    )?;
                    self.record_event(
                        "artifact_created",
                        item,
                        json!({"artifact_id": id.clone(), "name": name}),
                    )?;
                    serde_json::to_string(&json!({"artifact": self.get_artifact(&id)?}))?
                }
                ("GET", "/v1/artifacts") => serde_json::to_string(&json!({
                    "artifacts": self.list_artifacts(None)?
                }))?,
                ("GET", p) if p.starts_with("/v1/artifacts/") => {
                    let id = p.trim_start_matches("/v1/artifacts/");
                    serde_json::to_string(&json!({"artifact": self.get_artifact(id)?}))?
                }
                ("GET", "/v1/events") => serde_json::to_string(&json!({
                    "events": self.list_events(None, 100)?
                }))?,
                ("GET", "/v1/debug/bundle") => serde_json::to_string(&self.debug_bundle(None)?)?,
                ("GET", "/v1/search") => {
                    let q = query
                        .split('&')
                        .filter_map(|part| part.split_once('='))
                        .find(|(key, _)| *key == "q")
                        .map(|(_, value)| crate::util::url_decode(value))
                        .unwrap_or_default();
                    let results = self.search_results(&q)?;
                    serde_json::to_string(&json!({"results": results}))?
                }
                (_, "/health") => "{\"ok\":true}".to_string(),
                (m, p) => bail!("route not found: {m} {p}"),
            };
            Ok(body)
        })();
        let (status, body) = match body_result {
            Ok(body) => ("200 OK", body),
            Err(error) => {
                let message = error.to_string();
                let code = infer_error_code(&message);
                let status = match code {
                    "not_found" => "404 Not Found",
                    "internal_error" => "500 Internal Server Error",
                    _ => "400 Bad Request",
                };
                (
                    status,
                    serde_json::to_string(&json!({
                        "error": {
                            "code": code,
                            "message": message,
                            "details": {}
                        }
                    }))?,
                )
            }
        };
        let content_type = if path == "/review" || path == "/review/" {
            "text/html; charset=utf-8"
        } else {
            "application/json"
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\n{CORS_HEADERS}content-type: {content_type}\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        )?;
        Ok(())
    }

    /// Live Server-Sent Events stream: replays recent events, then follows
    /// the events table until the client disconnects.
    fn stream_events(&self, mut stream: TcpStream) -> Result<()> {
        // Streaming writes are spaced out; the per-connection write timeout
        // only needs to cover a single event or heartbeat write.
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
        write!(
            stream,
            "HTTP/1.1 200 OK\r\n{CORS_HEADERS}content-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n"
        )?;
        write!(stream, "retry: 5000\n\n")?;
        let mut last_id: i64 = 0;
        let replay = self.list_events(None, 100)?;
        if replay.is_empty() {
            write!(stream, "event: ready\ndata: {{\"ok\":true}}\n\n")?;
        }
        for event in replay.into_iter().rev() {
            last_id = write_sse_event(&mut stream, &event)?.max(last_id);
        }
        stream.flush()?;
        let mut idle_polls = 0u32;
        loop {
            let fresh = self.events_after(last_id)?;
            if fresh.is_empty() {
                idle_polls += 1;
                // Heartbeat comment roughly every 5 seconds keeps proxies and
                // clients aware the stream is alive and detects disconnects.
                if idle_polls >= 10 {
                    idle_polls = 0;
                    if write!(stream, ": keepalive\n\n").is_err() {
                        return Ok(());
                    }
                    if stream.flush().is_err() {
                        return Ok(());
                    }
                }
            } else {
                idle_polls = 0;
                for event in fresh {
                    match write_sse_event(&mut stream, &event) {
                        Ok(id) => last_id = id.max(last_id),
                        Err(_) => return Ok(()),
                    }
                }
                if stream.flush().is_err() {
                    return Ok(());
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

const CORS_HEADERS: &str = "access-control-allow-origin: *\r\naccess-control-allow-methods: GET, POST, OPTIONS\r\naccess-control-allow-headers: content-type\r\n";

fn write_sse_event(stream: &mut TcpStream, event: &Value) -> Result<i64> {
    let id = event.get("id").and_then(Value::as_i64).unwrap_or(0);
    write!(
        stream,
        "id: {id}\nevent: {}\ndata: {}\n\n",
        event
            .get("event_type")
            .and_then(Value::as_str)
            .unwrap_or("planr_event"),
        serde_json::to_string(event)?
    )?;
    Ok(id)
}
