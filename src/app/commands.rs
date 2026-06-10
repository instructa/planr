use super::recovery::ItemRecoveryInput;
use super::{App, ReviewAnnotationInput};
use crate::cli::{
    ApprovalCommand, ClientArg, CloseArgs, ContextCommand, DoctorArgs, ImportArgs, InstallCommand,
    ItemCommand, JsonOnlyArgs, LinkCommand, LogCommand, MapCommand, PickCommand, PlanCommand,
    ProjectCommand, PromptCommand, ReviewCommand, SearchArgs,
};
use crate::integrations::{agent_roles, install_snippet, mcp_json_config};
use crate::planpack::{build_plan_body, parse_plan_metadata, product_plan_files, project_pack_files};
use crate::util::{
    append_line, command_exists, format_item, format_project, json_array, now_string, print_json,
    short_id, worker_id, write_if_missing,
};
use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::{json, Value};
use slug::slugify;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

impl App {
    pub(crate) fn project(&self, command: ProjectCommand) -> Result<()> {
        match command {
            ProjectCommand::Init(args) => {
                let dirs = [
                    ".planr/project",
                    ".planr/plans/product",
                    ".planr/plans/build",
                    ".planr/reviews",
                ];
                for dir in dirs {
                    fs::create_dir_all(self.root.join(dir))?;
                }
                for (file, body) in project_pack_files() {
                    write_if_missing(
                        &self.root.join(".planr/project").join(file),
                        &body,
                        args.force,
                    )?;
                }
                let id = short_id("p");
                self.conn.execute(
                    "INSERT OR IGNORE INTO projects(id, name, root_path, description, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'active', datetime('now'), datetime('now'))",
                    params![id, args.name, self.root.to_string_lossy(), "Planr project"],
                )?;
                let project = self.default_project()?;
                self.record_event(
                    "project_initialized",
                    None,
                    json!({"project_id": project.id, "name": project.name}),
                )?;
                let client = args.client.map(|c| format!("{c:?}").to_lowercase());
                let clients: Vec<&str> = match client.as_deref() {
                    Some("all") => vec!["codex", "claude"],
                    Some(name) => vec![name],
                    None => Vec::new(),
                };
                let mut agent_paths = Vec::new();
                for target in clients {
                    for (relative, content) in agent_roles(target) {
                        let path = self.root.join(relative);
                        write_if_missing(&path, content, false)?;
                        agent_paths.push(path);
                    }
                }
                let out = json!({
                    "project": project,
                    "db": self.db_path,
                    "created_dirs": dirs,
                    "agents": agent_paths,
                    "client": client
                });
                self.emit(
                    out,
                    format!("initialized {} at {}", project.name, self.root.display()),
                )
            }
            ProjectCommand::Show(_) => {
                let project = self.default_project()?;
                self.emit(json!({"project": project}), format_project(&project))
            }
            ProjectCommand::List(_) => {
                let projects = self.list_projects()?;
                self.emit(
                    json!({"projects": projects}),
                    format!("{} project(s)", projects.len()),
                )
            }
            ProjectCommand::Delete(args) => {
                if !args.confirm {
                    bail!("refusing to delete without --confirm");
                }
                let changed = self.conn.execute("UPDATE projects SET status = 'deleted', updated_at = datetime('now') WHERE id = ?1 OR root_path = ?1", params![args.target])?;
                if args.with_files {
                    let planr = self.root.join(".planr");
                    if planr.exists() {
                        fs::remove_dir_all(planr)?;
                    }
                }
                self.emit(
                    json!({"updated": changed}),
                    format!("deleted records={changed}"),
                )
            }
        }
    }

    pub(crate) fn plan(&self, command: PlanCommand) -> Result<()> {
        match command {
            PlanCommand::New(args) => {
                let project = self.default_project()?;
                let slug = slugify(&args.title);
                let dir = self.root.join(".planr/plans/product").join(&slug);
                fs::create_dir_all(&dir)?;
                let files = product_plan_files(
                    &args.title,
                    args.platform.as_deref(),
                    args.ai,
                    args.backend,
                );
                for (name, body) in files {
                    write_if_missing(&dir.join(name), &body, false)?;
                }
                let manifest =
                    json!({"platform": args.platform, "ai": args.ai, "backend": args.backend});
                let plan =
                    self.upsert_plan(&project.id, "product", &dir, &args.title, &slug, manifest)?;
                self.emit(
                    json!({"plan": plan}),
                    format!("created product plan {}", plan.id),
                )
            }
            PlanCommand::Refine(args) => {
                let plan = self.get_plan(&args.id)?;
                let note = args
                    .note
                    .unwrap_or_else(|| "Refined assumptions and open decisions.".to_string());
                let path = PathBuf::from(&plan.path);
                let target = if path.is_dir() {
                    path.join("README.md")
                } else {
                    path
                };
                append_line(
                    &target,
                    &format!("\n## Refinement {}\n\n{}\n", now_string(), note),
                )?;
                self.rehash_plan(&plan.id)?;
                let updated = self.get_plan(&plan.id)?;
                self.emit(
                    json!({"plan": updated, "note": note}),
                    format!("refined {}", plan.id),
                )
            }
            PlanCommand::Split(args) => {
                let source = self.get_plan(&args.id)?;
                let project = self.default_project()?;
                let title = format!("{} - {}", source.title, args.slice);
                let slug = slugify(&title);
                let path = self
                    .root
                    .join(".planr/plans/build")
                    .join(format!("{slug}.plan.md"));
                let body = build_plan_body(&title, &source.id, &args.slice);
                write_if_missing(&path, &body, false)?;
                let plan = self.upsert_plan(
                    &project.id,
                    "build",
                    &path,
                    &title,
                    &slug,
                    json!({"source_plan": source.id, "slice": args.slice}),
                )?;
                self.emit(
                    json!({"plan": plan, "source": source.id}),
                    format!("created build plan {}", plan.id),
                )
            }
            PlanCommand::Check(args) => {
                let plan = self.get_plan(&args.id)?;
                let path = PathBuf::from(&plan.path);
                let mut warnings = Vec::new();
                if !path.exists() {
                    warnings.push("plan path missing".to_string());
                } else {
                    self.rehash_plan(&plan.id)?;
                    let (frontmatter, parse_status) = parse_plan_metadata(&path);
                    if parse_status != "ok" {
                        let detail = frontmatter["error"].as_str().unwrap_or("invalid frontmatter");
                        warnings.push(format!("frontmatter parse error: {detail}"));
                    }
                }
                let plan = self.get_plan(&args.id)?;
                let ok = warnings.is_empty();
                self.emit(
                    json!({"plan": plan, "ok": ok, "warnings": warnings}),
                    if ok {
                        "plan check passed".to_string()
                    } else {
                        "plan check failed".to_string()
                    },
                )
            }
            PlanCommand::Show(args) => {
                let plan = self.get_plan(&args.id)?;
                self.emit(
                    json!({"plan": plan}),
                    format!("{} [{}] {}", plan.id, plan.stage, plan.title),
                )
            }
            PlanCommand::List(args) => {
                let stage = args.stage.map(|s| format!("{s:?}").to_lowercase());
                let plans = self.list_plans(stage.as_deref())?;
                self.emit(json!({"plans": plans}), format!("{} plan(s)", plans.len()))
            }
            PlanCommand::Archive(args) => {
                self.conn.execute(
                    "UPDATE plans SET archived = 1, updated_at = datetime('now') WHERE id = ?1",
                    params![args.id],
                )?;
                self.emit(json!({"archived": args.id}), "plan archived".to_string())
            }
        }
    }

    pub(crate) fn map(&self, command: Option<MapCommand>) -> Result<()> {
        match command.unwrap_or(MapCommand::Show(JsonOnlyArgs { json: false })) {
            MapCommand::Show(_) => self.map_show(),
            MapCommand::Build(args) => {
                let plan = self.get_plan(&args.from)?;
                let created = self.seed_items_from_plan(&plan)?;
                self.promote_ready()?;
                let hint = (created.len() <= 1).then_some(
                    "created a single coarse item; run `planr item breakdown <item-id> --into \"...\"` before picking",
                );
                let mut message = format!("created {} map item(s)", created.len());
                if let Some(hint) = hint {
                    message.push_str("; ");
                    message.push_str(hint);
                }
                self.emit(json!({"created": created, "hint": hint}), message)
            }
            MapCommand::Lane(args) => {
                let lane = if args.critical {
                    self.critical_lane()?
                } else {
                    vec![]
                };
                self.emit(
                    json!({"critical": lane}),
                    format!("critical lane has {} item(s)", lane.len()),
                )
            }
            MapCommand::Pressure => {
                let pressure = self.pressure()?;
                self.emit(
                    json!({"pressure": pressure}),
                    "map pressure calculated".to_string(),
                )
            }
            MapCommand::Status => {
                let status = self.map_status_value()?;
                self.emit(status, "map status calculated".to_string())
            }
            MapCommand::Preview(args) => {
                let preview = self.preview_close_value(&args.close)?;
                self.emit(preview, "preview only".to_string())
            }
            MapCommand::Unlocks(args) => {
                let unlocks = self.would_unlock_items(&args.item_id)?;
                self.emit(
                    json!({"item_id": args.item_id, "would_unlock": unlocks}),
                    format!("{} item(s) would unlock", unlocks.len()),
                )
            }
            MapCommand::Lookahead(args) => {
                let lookahead = self.lookahead_value(args.from.as_deref(), args.limit)?;
                self.emit(lookahead, "map lookahead calculated".to_string())
            }
            MapCommand::Export(args) => {
                let data = self.export_value(true, true, None, &[])?;
                if args.format == "yaml" {
                    println!("{}", serde_yaml::to_string(&data)?);
                } else {
                    print_json(&data)?;
                }
                Ok(())
            }
            MapCommand::Import(args) => self.import(ImportArgs {
                file: args.file,
                preview: true,
                confirm: false,
            }),
        }
    }

    pub(crate) fn item(&self, command: ItemCommand) -> Result<()> {
        match command {
            ItemCommand::Create(args) => {
                let item = self.create_item(
                    None,
                    &args.title,
                    &args.description,
                    args.work_type.as_deref().unwrap_or("generic"),
                    None,
                )?;
                if let Some(after) = args.after {
                    self.add_link(&after, &item.id, "blocks")?;
                }
                if args.timeout_seconds.is_some()
                    || args.max_retries.is_some()
                    || args.retry_delay_ms.is_some()
                    || args.pre.is_some()
                    || args.post.is_some()
                    || args.retry_backoff != "exponential"
                {
                    self.configure_item_recovery(
                        &item.id,
                        ItemRecoveryInput {
                            timeout_seconds: args.timeout_seconds,
                            max_retries: args.max_retries,
                            retry_backoff: Some(args.retry_backoff.as_str()),
                            retry_delay_ms: args.retry_delay_ms,
                            pre_condition: args.pre.as_deref(),
                            post_condition: args.post.as_deref(),
                        },
                    )?;
                }
                self.promote_ready()?;
                let item = self.get_item(&item.id)?;
                self.emit(json!({"item": item}), format!("created item {}", item.id))
            }
            ItemCommand::Show(args) => {
                let item = self.get_item(&args.id)?;
                let logs = self.list_logs(Some(&args.id))?;
                self.emit(json!({"item": item, "logs": logs}), format_item(&item))
            }
            ItemCommand::Update(args) => {
                if let Some(title) = args.title {
                    self.conn.execute(
                        "UPDATE items SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                        params![title, args.id],
                    )?;
                }
                if let Some(description) = args.description {
                    self.conn.execute("UPDATE items SET description = ?1, updated_at = datetime('now') WHERE id = ?2", params![description, args.id])?;
                }
                let item = self.get_item(&args.id)?;
                self.emit(json!({"item": item}), "item updated".to_string())
            }
            ItemCommand::Breakdown(args) => {
                let parent = self.get_item(&args.id)?;
                let titles: Vec<_> = args
                    .into
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut created = Vec::new();
                let mut previous: Option<String> = None;
                for title in titles {
                    let child = self.create_item(
                        Some(&parent.id),
                        title,
                        &format!("Sub-item for {}", parent.title),
                        "generic",
                        parent.plan_path.as_deref(),
                    )?;
                    if let Some(prev) = previous {
                        self.add_link(&prev, &child.id, "blocks")?;
                    }
                    previous = Some(child.id.clone());
                    created.push(child);
                }
                self.conn.execute("UPDATE items SET status = 'blocked', updated_at = datetime('now') WHERE id = ?1", params![parent.id])?;
                self.promote_ready()?;
                self.emit(
                    json!({"items": created}),
                    format!("created {} child item(s)", created.len()),
                )
            }
            ItemCommand::Insert(args) => self.item_insert(args),
            ItemCommand::Amend(args) => self.item_amend(args),
            ItemCommand::Replan(args) => self.item_replan(args),
            ItemCommand::Cancel(args) => {
                if args.preview {
                    let item = self.get_item(&args.id)?;
                    return self.emit(json!({"would_cancel": item}), "preview only".to_string());
                }
                if !args.confirm {
                    bail!("refusing to cancel without --confirm or --preview");
                }
                self.conn.execute("UPDATE items SET status = 'cancelled', updated_at = datetime('now') WHERE id = ?1", params![args.id])?;
                self.promote_ready()?;
                self.emit(json!({"cancelled": args.id}), "item cancelled".to_string())
            }
        }
    }

    pub(crate) fn link(&self, command: LinkCommand) -> Result<()> {
        match command {
            LinkCommand::Add(args) => {
                self.add_link(&args.from_item, &args.to_item, &args.r#type)?;
                self.promote_ready()?;
                self.emit(
                    json!({"from": args.from_item, "to": args.to_item, "type": args.r#type}),
                    "link added".to_string(),
                )
            }
            LinkCommand::Remove(args) => {
                let changed = if let Some(kind) = args.r#type {
                    self.conn.execute(
                        "DELETE FROM links WHERE from_item = ?1 AND to_item = ?2 AND kind = ?3",
                        params![args.from_item, args.to_item, kind],
                    )?
                } else {
                    self.conn.execute(
                        "DELETE FROM links WHERE from_item = ?1 AND to_item = ?2",
                        params![args.from_item, args.to_item],
                    )?
                };
                self.promote_ready()?;
                self.emit(json!({"removed": changed}), "link removed".to_string())
            }
        }
    }

    pub(crate) fn pick(&self, command: Option<PickCommand>) -> Result<()> {
        match command {
            Some(PickCommand::Release(args)) => {
                let item = self.get_item(&args.item_id)?;
                let worker = worker_id();
                if !args.force && item.worker_id.as_deref() != Some(worker.as_str()) {
                    bail!(
                        "item is owned by {:?}; use --force to release",
                        item.worker_id
                    );
                }
                self.conn.execute("UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL, last_heartbeat_at = NULL, paused_at = NULL, updated_at = datetime('now') WHERE id = ?1 AND status IN ('picked','running')", params![args.item_id])?;
                self.record_event(
                    "pick_released",
                    Some(&args.item_id),
                    json!({"force": args.force}),
                )?;
                self.emit(
                    json!({"released": args.item_id}),
                    "pick released".to_string(),
                )
            }
            Some(PickCommand::Heartbeat(args)) => {
                let item_id = if let Some(id) = args.item_id {
                    id
                } else {
                    self.current_item_for_worker()?
                        .ok_or_else(|| anyhow!("no picked item for this worker"))?
                };
                self.heartbeat_item(&item_id)?;
                self.emit(
                    json!({"item": self.get_item(&item_id)?, "runtime": self.item_runtime(&item_id)?}),
                    "heartbeat recorded".to_string(),
                )
            }
            Some(PickCommand::Progress(args)) => {
                if !(0..=100).contains(&args.percent) {
                    bail!("progress percent must be between 0 and 100");
                }
                self.progress_item(&args.item_id, args.percent, args.note.as_deref())?;
                self.emit(
                    json!({"item": self.get_item(&args.item_id)?, "runtime": self.item_runtime(&args.item_id)?}),
                    "progress recorded".to_string(),
                )
            }
            Some(PickCommand::Pause(args)) => {
                self.pause_item(&args.item_id, args.note.as_deref())?;
                self.emit(
                    json!({"item": self.get_item(&args.item_id)?, "runtime": self.item_runtime(&args.item_id)?}),
                    "pick paused".to_string(),
                )
            }
            Some(PickCommand::Resume(args)) => {
                self.resume_item(&args.item_id)?;
                self.emit(
                    json!({"item": self.get_item(&args.item_id)?, "runtime": self.item_runtime(&args.item_id)?}),
                    "pick resumed".to_string(),
                )
            }
            Some(PickCommand::Stale(args)) => {
                let stale = self.stale_picks(args.older_than_seconds)?;
                if args.release {
                    for item in &stale {
                        if let Some(id) = item
                            .get("item")
                            .and_then(|value| value.get("id"))
                            .and_then(Value::as_str)
                        {
                            self.conn.execute(
                                "UPDATE items SET status = 'ready', worker_id = NULL, pick_token = NULL, last_heartbeat_at = NULL, paused_at = NULL, updated_at = datetime('now') WHERE id = ?1",
                                params![id],
                            )?;
                            self.record_event("stale_pick_released", Some(id), json!({}))?;
                        }
                    }
                    self.promote_ready()?;
                }
                self.emit(
                    json!({"stale": stale, "released": args.release}),
                    "stale picks inspected".to_string(),
                )
            }
            None => {
                if let Some((id, worker)) = self.pick_next_ready_item()? {
                    let item = self.get_item(&id)?;
                    let context = self.pick_context(&id)?;
                    return self.emit(
                        json!({"item": item, "worker_id": worker, "context": context}),
                        format!("picked {} {}", item.id, item.title),
                    );
                }
                self.emit(json!({"item": null}), "no ready item".to_string())
            }
        }
    }

    pub(crate) fn log(&self, command: LogCommand) -> Result<()> {
        match command {
            LogCommand::Add(args) => {
                let id = short_id("log");
                let run_id = if args.cmd.is_empty() && args.tests.is_empty() {
                    None
                } else {
                    Some(self.record_run(&args.item, &args.cmd, "closed")?)
                };
                self.conn.execute(
                    "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, files, commands, tests, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
                    params![
                        id,
                        self.default_project()?.id,
                        args.item,
                        run_id,
                        args.kind,
                        args.summary,
                        json_array(args.files.as_deref()),
                        serde_json::to_string(&args.cmd)?,
                        serde_json::to_string(&args.tests)?,
                    ],
                )?;
                self.index_search("log", &id, &args.summary, &args.summary, None)?;
                self.record_event(
                    "log_created",
                    Some(&args.item),
                    json!({"log_id": id, "kind": args.kind}),
                )?;
                self.emit(
                    json!({"log": self.get_log(&id)?}),
                    format!("created log {id}"),
                )
            }
            LogCommand::Show(args) => {
                let log = self.get_log(&args.id)?;
                self.emit(json!({"log": log}), format!("log {}", args.id))
            }
            LogCommand::List(args) => {
                let logs = self.list_logs(args.item.as_deref())?;
                self.emit(json!({"logs": logs}), format!("{} log(s)", logs.len()))
            }
        }
    }

    pub(crate) fn approval(&self, command: ApprovalCommand) -> Result<()> {
        match command {
            ApprovalCommand::Request(args) => {
                let item = self.get_item(&args.item_id)?;
                self.conn.execute(
                    "UPDATE items SET approval_status = 'requested', approval_requested_at = datetime('now'), approval_comment = ?1, approved_by = NULL, updated_at = datetime('now') WHERE id = ?2",
                    params![args.reason, item.id],
                )?;
                self.emit(
                    json!({"item": self.get_item(&item.id)?, "approval": self.item_approval(&item.id)?}),
                    "approval requested".to_string(),
                )
            }
            ApprovalCommand::Approve(args) => {
                let item = self.get_item(&args.item_id)?;
                self.conn.execute(
                    "UPDATE items SET approval_status = 'approved', approved_by = ?1, approval_comment = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![args.by, args.comment, item.id],
                )?;
                self.emit(
                    json!({"item": self.get_item(&item.id)?, "approval": self.item_approval(&item.id)?}),
                    "approval recorded".to_string(),
                )
            }
            ApprovalCommand::Deny(args) => {
                let item = self.get_item(&args.item_id)?;
                self.conn.execute(
                    "UPDATE items SET approval_status = 'denied', approved_by = ?1, approval_comment = ?2, updated_at = datetime('now') WHERE id = ?3",
                    params![args.by, args.comment, item.id],
                )?;
                self.emit(
                    json!({"item": self.get_item(&item.id)?, "approval": self.item_approval(&item.id)?}),
                    "approval denied".to_string(),
                )
            }
            ApprovalCommand::List(args) => {
                let approvals = self.list_approvals(args.open)?;
                self.emit(
                    json!({"approvals": approvals}),
                    format!("{} approval item(s)", approvals.len()),
                )
            }
        }
    }

    pub(crate) fn close(&self, args: CloseArgs) -> Result<()> {
        let item_id = if let Some(id) = args.item_id {
            id
        } else {
            self.current_item_for_worker()?
                .ok_or_else(|| anyhow!("no picked item for this worker"))?
        };
        // Reconcile gate state first so a parent whose children are already
        // settled is closable instead of stuck in `blocked`.
        self.promote_ready()?;
        self.ensure_can_close(&item_id)?;
        self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item_id])?;
        let log_id = short_id("log");
        self.conn.execute(
            "INSERT INTO logs(id, project_id, item_id, kind, summary, created_at) VALUES (?1, ?2, ?3, 'completion', ?4, datetime('now'))",
            params![log_id, self.default_project()?.id, item_id, args.summary],
        )?;
        self.promote_ready()?;
        self.record_event(
            "item_closed",
            Some(&item_id),
            json!({"log_id": log_id, "summary": args.summary}),
        )?;
        let next = if args.next {
            let before = self.json;
            let _ = before;
            self.pick(None)?;
            None::<Value>
        } else {
            None
        };
        self.emit(
            json!({"closed": item_id, "log_id": log_id, "next": next}),
            "item closed".to_string(),
        )
    }

    pub(crate) fn review(&self, command: ReviewCommand) -> Result<()> {
        match command {
            ReviewCommand::Request(args) => {
                let target = self.get_item(&args.item_id)?;
                let review = self.create_item(
                    None,
                    &format!("Review {}", target.title),
                    "Review item against plan, logs, diff, and verification.",
                    "review",
                    target.plan_path.as_deref(),
                )?;
                self.add_link(&review.id, &target.id, "reviews")?;
                self.promote_ready()?;
                let review = self.get_item(&review.id)?;
                self.record_event(
                    "review_requested",
                    Some(&target.id),
                    json!({"review_id": review.id.clone()}),
                )?;
                self.emit(json!({"review": review}), "review requested".to_string())
            }
            ReviewCommand::Annotate(args) => {
                let annotation = self.add_review_annotation(ReviewAnnotationInput {
                    item_id: &args.item_id,
                    message: &args.message,
                    severity: &args.severity,
                    author: args.author.as_deref(),
                    file: args.file.as_deref(),
                    line: args.line,
                    source: "cli",
                })?;
                self.emit(
                    json!({"annotation": annotation}),
                    "review annotation added".to_string(),
                )
            }
            ReviewCommand::Ingest(args) => {
                let raw = if args.stdin {
                    let mut input = String::new();
                    io::stdin().read_to_string(&mut input)?;
                    input
                } else if let Some(path) = args.from {
                    fs::read_to_string(path)?
                } else {
                    bail!("review ingest requires --from PATH or --stdin");
                };
                let feedback: Value = serde_json::from_str(&raw)?;
                let result = self.ingest_review_feedback(&args.item_id, feedback, "cli")?;
                self.emit(result, "review feedback ingested".to_string())
            }
            ReviewCommand::Artifact(args) => {
                let artifact =
                    self.write_review_artifact(&args.review_item_id, None, &[], &[], args.out)?;
                self.emit(
                    json!({"artifact": artifact}),
                    "review artifact written".to_string(),
                )
            }
            ReviewCommand::Evidence(args) => {
                let pr_context = args
                    .pr_url
                    .as_deref()
                    .map(|url| self.record_pr_url(&args.item_id, url))
                    .transpose()?;
                self.emit(
                    json!({
                        "evidence": self.review_evidence_value(&args.item_id)?,
                        "pr_context": pr_context
                    }),
                    "review evidence collected".to_string(),
                )
            }
            ReviewCommand::Close(args) => {
                let verdict = args.verdict.as_str();
                let result =
                    self.close_review_item(&args.review_item_id, verdict, args.findings, "cli")?;
                self.emit(result, "review closed".to_string())
            }
            ReviewCommand::List(args) => {
                let status_filter = if args.open { Some("closed") } else { None };
                let reviews = self.list_items_by_type("review", status_filter)?;
                self.emit(
                    json!({"reviews": reviews}),
                    format!("{} review item(s)", reviews.len()),
                )
            }
            ReviewCommand::Show(args) => {
                let item = self.get_item(&args.id)?;
                let logs = self.list_logs(Some(&args.id))?;
                self.emit(
                    json!({"review": item, "logs": logs}),
                    "review detail".to_string(),
                )
            }
        }
    }

    pub(crate) fn context(&self, command: ContextCommand) -> Result<()> {
        match command {
            ContextCommand::Add(args) => {
                let id = short_id("ctx");
                self.conn.execute(
                    "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
                    params![id, self.default_project()?.id, args.item, worker_id(), args.tag, args.text, "[]"],
                )?;
                self.index_search("context", &id, &args.tag, &args.text, None)?;
                self.record_event(
                    "context_created",
                    args.item.as_deref(),
                    json!({"context_id": id.clone(), "tag": args.tag}),
                )?;
                self.emit(
                    json!({"context": self.get_context(&id)?}),
                    format!("added note {id}"),
                )
            }
            ContextCommand::List(args) => {
                let values = self.list_contexts(args.item.as_deref())?;
                self.emit(
                    json!({"contexts": values}),
                    format!("{} note(s)", values.len()),
                )
            }
        }
    }

    pub(crate) fn search(&self, args: SearchArgs) -> Result<()> {
        let results = self.search_results(&args.query)?;
        self.emit(
            json!({"results": results}),
            format!("{} result(s)", results.len()),
        )
    }

    pub(crate) fn doctor(&self, args: DoctorArgs) -> Result<()> {
        let clients = match args.client.unwrap_or(ClientArg::All) {
            ClientArg::Codex => vec!["codex"],
            ClientArg::Claude => vec!["claude"],
            ClientArg::Cursor => vec!["cursor"],
            ClientArg::All => vec!["codex", "claude", "cursor"],
        };
        let checks: Vec<_> = clients
            .into_iter()
            .map(|client| {
                let installed = command_exists(client);
                json!({
                    "client": client,
                    "status": if installed { "pass" } else { "not_installed" },
                    "installed": installed,
                    "install": format!("planr install {client} --dry-run")
                })
            })
            .collect();
        let data = json!({
            "db": self.db_path,
            "db_status": if self.db_path.exists() { "pass" } else { "warning" },
            "project": self.default_project().ok(),
            "clients": checks,
            "mcp": {"command": "planr mcp"}
        });
        if self.json {
            print_json(&data)
        } else {
            println!("doctor complete");
            println!(
                "database: {}",
                data["db_status"].as_str().unwrap_or("warning")
            );
            for check in data["clients"].as_array().unwrap_or(&Vec::new()) {
                println!(
                    "{}: {} ({})",
                    check["client"].as_str().unwrap_or("client"),
                    check["status"].as_str().unwrap_or("warning"),
                    check["install"].as_str().unwrap_or("")
                );
            }
            println!("mcp: planr mcp");
            Ok(())
        }
    }

    pub(crate) fn install(&self, command: InstallCommand) -> Result<()> {
        let (client, dry_run) = match command {
            InstallCommand::Codex(args) => ("codex", args.dry_run),
            InstallCommand::Claude(args) => ("claude", args.dry_run),
            InstallCommand::Cursor(args) => ("cursor", args.dry_run),
        };
        let snippet = install_snippet(client, &self.db_path);
        if dry_run {
            println!("{snippet}");
            return Ok(());
        }
        let mut agent_paths = Vec::new();
        for (relative, content) in agent_roles(client) {
            let path = self.root.join(relative);
            write_if_missing(&path, content, false)?;
            agent_paths.push(path);
        }
        match client {
            "codex" => {
                let path = self.root.join(".planr/integrations/codex-mcp.toml");
                write_if_missing(&path, &snippet, true)?;
                self.emit(
                    json!({"client": client, "path": path, "agents": agent_paths}),
                    "codex integration written".to_string(),
                )
            }
            "claude" => {
                let path = self.root.join(".mcp.json");
                write_if_missing(&path, &mcp_json_config(&self.db_path), true)?;
                self.emit(
                    json!({"client": client, "path": path, "agents": agent_paths}),
                    "claude integration written".to_string(),
                )
            }
            "cursor" => {
                let path = self.root.join(".cursor/mcp.json");
                write_if_missing(&path, &mcp_json_config(&self.db_path), true)?;
                self.emit(
                    json!({"client": client, "path": path}),
                    "cursor integration written".to_string(),
                )
            }
            _ => bail!("unknown client: {client}"),
        }
    }

    pub(crate) fn prompt(&self, command: PromptCommand) -> Result<()> {
        let (mode, client) = match command {
            PromptCommand::Cli(args) => ("cli", args.client),
            PromptCommand::Mcp(args) => ("mcp", args.client),
            PromptCommand::Http(args) => ("http", args.client),
        };
        let client = client
            .map(|value| format!("{value:?}").to_lowercase())
            .unwrap_or_else(|| "generic".to_string());
        let prompt = match mode {
            "cli" => format!(
                "Use Planr as the local source of truth for planning and execution. Start with `planr project show --json`, inspect `planr map status --json`, pick work with `planr pick --json`, log evidence with `planr log add`, request and close reviews with `planr review ...`, and close only after `planr map preview --close <item-id>` is clean. Use database `{}` when an explicit DB path is needed. Target client: {client}.",
                self.db_path.display()
            ),
            "mcp" => format!(
                "Configure a project-scoped MCP server with command `planr --db {} mcp`. Use `planr install codex|claude|cursor --dry-run` for client-specific snippets, or this generic JSON:\n{}",
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
