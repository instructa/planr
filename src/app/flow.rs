use super::App;
use crate::cli::DoneArgs;
use crate::route_audit::{RouteObservation, load_route_observation};
use crate::util::{short_id, worker_id};
use anyhow::{Result, anyhow, bail};
use rusqlite::params;
use serde_json::{Value, json};

/// The profile a run actually executed on: explicit input wins, then the
/// `PLANR_PROFILE` environment variable (role files rendered from the
/// registry can export it), else none — and none means no comparison and
/// no event, never a guess.
fn effective_profile(explicit: Option<&str>) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            std::env::var("PLANR_PROFILE")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

pub(crate) struct LogInput<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) summary: &'a str,
    pub(crate) files: &'a [String],
    pub(crate) commands: &'a [String],
    pub(crate) tests: &'a [String],
    pub(crate) source: Option<&'a str>,
    /// Registry profile the work actually executed on, when the worker
    /// knows it (`--profile` flag; `PLANR_PROFILE` env is the fallback).
    pub(crate) profile: Option<&'a str>,
    pub(crate) route_observation: Option<&'a RouteObservation>,
}

/// Owner of the compound work flow: evidence logging, the close transition,
/// review requests, and the `done` command that chains them. CLI, HTTP, and
/// MCP surfaces call these instead of duplicating the steps.
impl App {
    /// Single owner for writing evidence logs. Logging from the pick owner
    /// also refreshes the runtime heartbeat: evidence is a liveness signal,
    /// so agents do not need a separate `pick heartbeat` call.
    pub(crate) fn add_log_entry(&self, input: LogInput<'_>) -> Result<String> {
        if input.route_observation.is_some() && input.commands.is_empty() && input.tests.is_empty()
        {
            bail!("route audit requires a recorded run with at least one command or test");
        }
        let id = short_id("log");
        let profile = effective_profile(input.profile);
        let run_id = if input.commands.is_empty() && input.tests.is_empty() {
            None
        } else {
            Some(self.record_run(
                input.item_id,
                input.commands,
                "closed",
                profile.as_deref(),
                input.route_observation,
            )?)
        };
        if let Some(run_id) = run_id.as_deref() {
            if let Some(actual_profile) = profile.as_deref() {
                self.observe_route_compliance(input.item_id, run_id, actual_profile, input.kind)?;
            }
            // Client compliance needs no profile: the observed host comes
            // from the environment, so it also audits runs whose worker
            // never reported a profile.
            self.observe_client_compliance(input.item_id, run_id, input.kind)?;
            if let Some(observation) = input.route_observation {
                self.record_route_observation_events(input.item_id, run_id, observation)?;
            }
        }
        self.conn.execute(
            "INSERT INTO logs(id, project_id, item_id, run_id, kind, summary, files, commands, tests, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            params![
                id,
                self.default_project()?.id,
                input.item_id,
                run_id,
                input.kind,
                input.summary,
                serde_json::to_string(
                    &input.files
                        .iter()
                        .map(|file| file.trim())
                        .filter(|file| !file.is_empty())
                        .collect::<Vec<_>>(),
                )?,
                serde_json::to_string(input.commands)?,
                serde_json::to_string(input.tests)?,
            ],
        )?;
        self.index_search("log", &id, input.summary, input.summary, None)?;
        let mut event = json!({"log_id": id, "kind": input.kind});
        if let Some(source) = input.source {
            event["source"] = json!(source);
        }
        self.record_event("log_created", Some(input.item_id), event)?;
        self.conn.execute(
            "UPDATE items SET status = CASE WHEN status = 'picked' THEN 'running' ELSE status END, last_heartbeat_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND worker_id = ?2 AND status IN ('picked','running','in_review')",
            params![input.item_id, worker_id()],
        )?;
        Ok(id)
    }

    /// Single owner for the close transition. Reconciles gate state first so
    /// a parent whose children are already settled is closable instead of
    /// stuck in `blocked`, then validates, closes, and promotes unlocked work.
    pub(crate) fn close_item_core(
        &self,
        item_id: &str,
        summary: &str,
        write_log: bool,
    ) -> Result<Option<String>> {
        self.promote_ready()?;
        self.ensure_can_close(item_id)?;
        self.conn.execute("UPDATE items SET status = 'closed', completed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1", params![item_id])?;
        let log_id = if write_log {
            Some(self.add_log_entry(LogInput {
                item_id,
                kind: "completion",
                summary,
                files: &[],
                commands: &[],
                tests: &[],
                source: None,
                profile: None,
                route_observation: None,
            })?)
        } else {
            None
        };
        self.promote_ready()?;
        self.record_event(
            "item_closed",
            Some(item_id),
            json!({"log_id": log_id, "summary": summary}),
        )?;
        Ok(log_id)
    }

    /// Completion-time context shared by `done` and `close`: what the
    /// settlement unlocked, the post condition to verify, and an evidence
    /// hint when downstream work exists but no commands/tests were logged.
    pub(crate) fn settlement_extras(
        &self,
        item_id: &str,
        ready_before: &std::collections::HashSet<String>,
        has_evidence: bool,
    ) -> Result<Value> {
        let unlocked = self.unlocked_since(ready_before)?;
        let post_condition = self.item_conditions(item_id)?["post"]
            .as_str()
            .map(ToOwned::to_owned);
        let downstream: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM links WHERE from_item = ?1 AND kind = 'blocks'",
            params![item_id],
            |row| row.get(0),
        )?;
        let hint = (!has_evidence && downstream > 0).then(|| {
            format!(
                "downstream items depend on {item_id} but this log has no --cmd/--tests evidence; attach it with `planr log add {item_id} --summary ... --cmd ...`"
            )
        });
        Ok(json!({"unlocked": unlocked, "post_condition": post_condition, "hint": hint}))
    }

    pub(crate) fn settlement_extras_human(extras: &Value) -> String {
        let mut human = Self::unlocked_human(
            extras["unlocked"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
        if let Some(post) = extras["post_condition"].as_str() {
            human.push_str(&format!("\npost condition to verify: {post}"));
        }
        if let Some(hint) = extras["hint"].as_str() {
            human.push_str(&format!("\nhint: {hint}"));
        }
        human
    }

    pub(crate) fn progress_human(progress: &Value) -> String {
        let ready = progress["counts"]["ready"].as_i64().unwrap_or(0);
        format!(
            " [{}/{} settled · {} ready]",
            progress["settled"], progress["total"], ready
        )
    }

    /// `planr done`: completion log, then review request or close, then an
    /// optional next pick — one command instead of three, same evidence.
    pub(crate) fn done(&self, args: DoneArgs) -> Result<()> {
        let route_observation = args
            .route_audit
            .as_deref()
            .map(load_route_observation)
            .transpose()?;
        let item_id = if let Some(id) = args.item_id {
            id
        } else {
            self.current_item_for_worker()?
                .ok_or_else(|| anyhow!("no picked item for this worker"))?
        };
        let ready_before = self.ready_item_ids()?;
        // A never-picked ready item is adopted (leased retroactively) so the
        // completion carries worker attribution and the review transition
        // cannot be skipped.
        let adopted = self.adopt_ready_item(&item_id)?;
        let log_id = self.add_log_entry(LogInput {
            item_id: &item_id,
            kind: "completion",
            summary: &args.summary,
            files: &args.files,
            commands: &args.cmd,
            tests: &args.tests,
            source: None,
            profile: args.profile.as_deref(),
            route_observation: route_observation.as_ref(),
        })?;
        let review = if args.review {
            Some(self.request_review_for(&item_id)?)
        } else {
            self.close_item_core(&item_id, &args.summary, false)?;
            None
        };
        let extras = self.settlement_extras(
            &item_id,
            &ready_before,
            !args.cmd.is_empty() || !args.tests.is_empty(),
        )?;
        let item = self.get_item(&item_id)?;
        let next = if args.next {
            // A worker must not pick the review it just requested:
            // maker and checker stay separate.
            Some(self.next_pick_value(review.as_ref().map(|r| r.id.as_str()), None, None)?)
        } else {
            // Without --next, name the exact follow-up command (plan-scoped
            // when the item belongs to a plan) so the settlement output
            // still ends in an action, not a dead end.
            let plan_flag = item
                .plan_path
                .as_deref()
                .and_then(|path| self.plan_id_for_path(path).transpose())
                .transpose()?
                .map(|id| format!(" --plan {id}"))
                .unwrap_or_default();
            let command = if review.is_some() {
                format!("planr pick{plan_flag} --work-type review --json")
            } else {
                format!("planr pick{plan_flag} --json")
            };
            Some(json!(command))
        };
        let mut human = if let Some(review) = &review {
            format!(
                "logged {item_id} and requested review {}; {item_id} is {}",
                review.id, item.status
            )
        } else {
            format!("logged and closed {item_id}")
        };
        if adopted {
            human.push_str(" (item was never picked; adopted under this worker)");
        }
        if let Some(next) = &next {
            if let Some(command) = next.as_str() {
                human.push_str(&format!("; next: {command}"));
            } else {
                match next["item"]["id"].as_str() {
                    Some(next_id) => human.push_str(&format!("; picked {next_id}")),
                    None => human.push_str("; no ready item"),
                }
            }
        }
        let progress = self.progress_value()?;
        human.push_str(&Self::progress_human(&progress));
        human.push_str(&Self::settlement_extras_human(&extras));
        self.emit(
            json!({
                "item": item,
                "log_id": log_id,
                "review": review,
                "closed": if args.review { Value::Null } else { json!(item_id) },
                "unlocked": extras["unlocked"],
                "post_condition": extras["post_condition"],
                "hint": extras["hint"],
                "next": next,
                "remaining": progress,
            }),
            human,
        )
    }
}
