use super::App;
use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::process::Command as StdCommand;

impl App {
    pub(crate) fn record_pr_url(&self, item_id: &str, pr_url: &str) -> Result<Value> {
        self.get_item(item_id)?;
        let id = crate::util::short_id("ctx");
        self.conn.execute(
            "INSERT INTO contexts(id, project_id, item_id, worker_id, kind, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, 'pr_url', ?5, ?6, datetime('now'))",
            params![
                id,
                self.default_project()?.id,
                item_id,
                crate::util::worker_id(),
                pr_url,
                json!(["review", "git", "pr"]).to_string()
            ],
        )?;
        self.index_search("context", &id, "pr_url", pr_url, None)?;
        self.record_event(
            "review_pr_url_recorded",
            Some(item_id),
            json!({"context_id": id.clone(), "pr_url": pr_url}),
        )?;
        self.get_context(&id)
    }

    pub(crate) fn review_evidence_value(&self, item_id: &str) -> Result<Value> {
        let item = self.get_item(item_id)?;
        let logged_files = self.logged_files_for_item(item_id)?;
        let artifact_files = self
            .list_artifacts(Some(item_id))?
            .into_iter()
            .filter_map(|artifact| {
                artifact
                    .get("path")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect::<BTreeSet<_>>();
        let owned_files = logged_files
            .union(&artifact_files)
            .cloned()
            .collect::<BTreeSet<_>>();
        let pr_urls = self
            .list_contexts(Some(item_id))?
            .into_iter()
            .filter(|context| context.get("kind").and_then(Value::as_str) == Some("pr_url"))
            .filter_map(|context| {
                context
                    .get("content")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>();
        let git = self.git_evidence(&owned_files);
        Ok(json!({
            "item": item,
            "git": git,
            "provenance": {
                "logged_files": logged_files,
                "artifact_files": artifact_files,
                "agent_owned_files": owned_files,
                "pr_urls": pr_urls,
            },
            "dirty_worktree_safety": {
                "source_content_included": false,
                "unrelated_changes_are_agent_owned": false,
                "requires_log_or_artifact_provenance": true,
            }
        }))
    }

    fn logged_files_for_item(&self, item_id: &str) -> Result<BTreeSet<String>> {
        let mut files = BTreeSet::new();
        for log in self.list_logs(Some(item_id))? {
            collect_file_values(log.get("files"), &mut files);
        }
        Ok(files)
    }

    fn git_evidence(&self, owned_files: &BTreeSet<String>) -> Value {
        let inside = StdCommand::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&self.root)
            .output();
        if !matches!(inside, Ok(output) if output.status.success()) {
            return json!({
                "available": false,
                "message": "No Git worktree detected.",
                "changed_files": [],
                "scoped_files": [],
                "unrelated_dirty_files": [],
                "diff_stat": "",
                "source_content_included": false,
            });
        }
        let changed_files = self.git_changed_files();
        let scoped_files = changed_files
            .intersection(owned_files)
            .cloned()
            .collect::<BTreeSet<_>>();
        let unrelated_dirty_files = changed_files
            .difference(&scoped_files)
            .cloned()
            .collect::<BTreeSet<_>>();
        let diff_stat = if scoped_files.is_empty() {
            String::new()
        } else {
            let mut command = StdCommand::new("git");
            command.args(["diff", "--stat", "--"]);
            for file in &scoped_files {
                command.arg(file);
            }
            command
                .current_dir(&self.root)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                .unwrap_or_default()
        };
        json!({
            "available": true,
            "message": if owned_files.is_empty() {
                "No item log or artifact file provenance; dirty files are treated as unrelated."
            } else {
                "Git evidence scoped to files named by item logs or artifacts."
            },
            "changed_files": changed_files,
            "scoped_files": scoped_files,
            "unrelated_dirty_files": unrelated_dirty_files,
            "diff_stat": diff_stat,
            "source_content_included": false,
        })
    }

    fn git_changed_files(&self) -> BTreeSet<String> {
        StdCommand::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(parse_porcelain_path)
                    .filter(|path| !is_planr_runtime_db_file(path))
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default()
    }
}

fn is_planr_runtime_db_file(path: &str) -> bool {
    matches!(
        path,
        ".planr/planr.sqlite" | ".planr/planr.sqlite-shm" | ".planr/planr.sqlite-wal"
    )
}

fn collect_file_values(value: Option<&Value>, files: &mut BTreeSet<String>) {
    match value {
        Some(Value::Array(values)) => {
            for value in values {
                if let Some(path) = value.as_str().filter(|path| !path.trim().is_empty()) {
                    files.insert(path.to_string());
                }
            }
        }
        Some(Value::String(path)) if !path.trim().is_empty() => {
            files.insert(path.to_string());
        }
        _ => {}
    }
}

fn parse_porcelain_path(line: &str) -> Option<String> {
    if line.len() < 4 {
        return None;
    }
    let path = &line[3..];
    let path = path.split(" -> ").last().unwrap_or(path).trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}
