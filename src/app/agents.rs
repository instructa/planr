//! CLI and MCP surface for the agent profile registry and per-item
//! routing. The `*_value` functions own the JSON shapes; the CLI adds
//! human rendering on top and MCP tools return the same values, so both
//! surfaces stay contract-identical by construction.

use super::App;
use crate::agents::{
    AgentRegistry, REGISTRY_RELATIVE_PATH, RegistryLoad, RoutingFacts, load_registry,
    resolve_route, validation_warnings,
};
use crate::cli::{AgentsCommand, ClientArg, ItemRouteArgs};
use crate::rolefiles::{
    GENERATED_FROM_HEADER, agent_roles, render_claude_role, render_codex_role, render_cursor_role,
};
use anyhow::{Result, bail};
use rusqlite::params;
use serde_json::{Value, json};

impl App {
    pub(crate) fn agents(&self, command: AgentsCommand) -> Result<()> {
        match command {
            AgentsCommand::List(_) => {
                let (value, human) = self.agents_list_value()?;
                self.emit(value, human)
            }
            AgentsCommand::Check => match load_registry(&self.root) {
                RegistryLoad::Missing => self.emit(
                    json!({"ok": true, "reason": "missing", "warnings": []}),
                    format!("no registry at {REGISTRY_RELATIVE_PATH}; nothing to check"),
                ),
                // Parse failure is the one non-zero exit; the top-level
                // error handler renders it (structured JSON under --json).
                RegistryLoad::Degraded { error } => {
                    bail!("agent registry parse failed: {error}")
                }
                RegistryLoad::Loaded(registry) => {
                    let warnings = validation_warnings(&registry);
                    let human = if warnings.is_empty() {
                        "agent registry check passed".to_string()
                    } else {
                        let mut human =
                            format!("agent registry parses; {} warning(s)", warnings.len());
                        for warning in &warnings {
                            human.push_str(&format!("\n  {warning}"));
                        }
                        human
                    };
                    self.emit(json!({"ok": true, "warnings": warnings}), human)
                }
            },
            AgentsCommand::Init(args) => self.agents_init(args),
        }
    }

    /// The registry listing shared by `planr agents list` and the
    /// `planr_agents_list` MCP tool: resolved registry (or the
    /// missing/degraded reason) plus validation warnings.
    pub(crate) fn agents_list_value(&self) -> Result<(Value, String)> {
        Ok(match load_registry(&self.root) {
            RegistryLoad::Missing => (
                json!({"registry": null, "reason": "missing", "path": REGISTRY_RELATIVE_PATH}),
                format!(
                    "no agent registry; declare profiles and routes in {REGISTRY_RELATIVE_PATH} to get routing recommendations in pick packets"
                ),
            ),
            RegistryLoad::Degraded { error } => (
                json!({"registry": null, "reason": "degraded", "path": REGISTRY_RELATIVE_PATH, "error": error}),
                format!("agent registry unusable: {error}"),
            ),
            RegistryLoad::Loaded(registry) => {
                let warnings = validation_warnings(&registry);
                let mut human = format!(
                    "{} profile(s), {} route(s), default route: {}",
                    registry.profiles.len(),
                    registry.routes.len(),
                    registry
                        .route_default
                        .as_ref()
                        .map_or("none", |default| default.profile.as_str())
                );
                for (id, profile) in &registry.profiles {
                    human.push_str(&format!(
                        "\n  {id}: {} {}{}{}{}",
                        profile.client,
                        profile.model,
                        profile
                            .effort
                            .as_deref()
                            .map(|effort| format!(" effort={effort}"))
                            .unwrap_or_default(),
                        profile
                            .cost_tier
                            .as_deref()
                            .map(|tier| format!(" tier={tier}"))
                            .unwrap_or_default(),
                        profile
                            .skill
                            .as_deref()
                            .map(|skill| format!(" skill={skill}"))
                            .unwrap_or_default(),
                    ));
                }
                for warning in &warnings {
                    human.push_str(&format!("\n  warning: {warning}"));
                }
                (json!({"registry": registry, "warnings": warnings}), human)
            }
        })
    }

    /// The advisory routing block for a pick packet, or None when no
    /// registry exists, it is degraded, or nothing resolves — the packet
    /// simply omits `routing` in all of those cases.
    pub(crate) fn routing_value_for_item(&self, item_id: &str) -> Result<Option<Value>> {
        let RegistryLoad::Loaded(registry) = load_registry(&self.root) else {
            return Ok(None);
        };
        let facts = self.item_route_facts(item_id)?;
        Ok(resolve_route(&facts.as_routing_facts(), &registry).map(|routing| json!(routing)))
    }

    /// Advisory declared-vs-actual check at evidence time: every host has
    /// a silent override path (env preemption, plan/admin policy, org
    /// allowlists, full-history forks), so the recorded profile is the
    /// only trustworthy signal. A mismatch emits exactly one
    /// `route_mismatch_observed` event — it never blocks logging, review,
    /// or closure. No declared route (no registry, nothing resolves)
    /// means no comparison and no event.
    pub(crate) fn observe_route_compliance(
        &self,
        item_id: &str,
        run_id: &str,
        actual_profile: &str,
    ) -> Result<()> {
        let Some(routing) = self.routing_value_for_item(item_id)? else {
            return Ok(());
        };
        let Some(declared) = routing["profile"].as_str() else {
            return Ok(());
        };
        if declared != actual_profile {
            self.record_event(
                "route_mismatch_observed",
                Some(item_id),
                json!({
                    "declared_profile": declared,
                    "actual_profile": actual_profile,
                    "run_id": run_id,
                    "matched_selector": routing["matched_selector"],
                }),
            )?;
        }
        Ok(())
    }

    /// Compares the host a run observably executed under (env-detected,
    /// not self-declared) with the client the item's declared route
    /// names. Closes the audit blindspot where profile self-report
    /// masks a client-level deviation — e.g. Cursor subagents standing
    /// in for a declared Codex profile. Advisory like everything here:
    /// no observation, no registry, or no declared client means no
    /// comparison and no event.
    pub(crate) fn observe_client_compliance(&self, item_id: &str, run_id: &str) -> Result<()> {
        let Some(observed) = crate::util::observed_client() else {
            return Ok(());
        };
        let Some(routing) = self.routing_value_for_item(item_id)? else {
            return Ok(());
        };
        let Some(declared) = routing["client"].as_str() else {
            return Ok(());
        };
        if !client_vocab_matches(declared, &observed) {
            self.record_event(
                "client_mismatch_observed",
                Some(item_id),
                json!({
                    "declared_client": declared,
                    "observed_client": observed,
                    "declared_profile": routing["profile"],
                    "run_id": run_id,
                    "matched_selector": routing["matched_selector"],
                }),
            )?;
        }
        Ok(())
    }

    /// The `routing` section of `trace item`: the declared route next to
    /// every recorded run's actual client/profile with a mismatch marker.
    /// Returns None when there is nothing to say (no declared route and
    /// no profiled runs), keeping traces byte-identical to the
    /// pre-routing output for projects that never opted in.
    pub(crate) fn trace_routing_value(&self, item_id: &str) -> Result<Option<Value>> {
        let declared = self.routing_value_for_item(item_id)?;
        let runs = self.item_runs(item_id)?;
        let any_profiled = runs.iter().any(|run| run["profile"].is_string());
        if declared.is_none() && !any_profiled {
            return Ok(None);
        }
        let declared_profile = declared
            .as_ref()
            .and_then(|routing| routing["profile"].as_str())
            .map(ToOwned::to_owned);
        let declared_client = declared
            .as_ref()
            .and_then(|routing| routing["client"].as_str())
            .map(ToOwned::to_owned);
        let runs: Vec<Value> = runs
            .into_iter()
            .map(|mut run| {
                // Advisory markers: only comparable when both sides exist.
                run["mismatch"] = match (declared_profile.as_deref(), run["profile"].as_str()) {
                    (Some(declared), Some(actual)) => json!(declared != actual),
                    _ => Value::Null,
                };
                if let (Some(declared), Some(observed)) =
                    (declared_client.as_deref(), run["observed_client"].as_str())
                {
                    run["client_mismatch"] = json!(!client_vocab_matches(declared, observed));
                }
                run
            })
            .collect();
        let mismatches = runs
            .iter()
            .filter(|run| run["mismatch"] == json!(true))
            .count();
        Ok(Some(json!({
            "declared": declared,
            "runs": runs,
            "mismatches": mismatches,
        })))
    }

    /// The `doctor` registry block. Never fails the command: absent is
    /// informational, a parse failure is a warning with the parser's line
    /// context, and a loaded registry reports counts, validation
    /// warnings, and drift of rendered role files (a file whose
    /// generated-from header no longer matches what the current registry
    /// would render). Files without the header are the user's and are
    /// reported as `manual`, never flagged.
    pub(crate) fn registry_doctor_value(&self) -> Result<Value> {
        let registry = match load_registry(&self.root) {
            RegistryLoad::Missing => {
                return Ok(json!({
                    "path": REGISTRY_RELATIVE_PATH,
                    "status": "absent",
                    "hint": "no agent registry; create .planr/agents.toml to declare model routing (docs/MODEL_ROUTING.md)",
                }));
            }
            RegistryLoad::Degraded { error } => {
                return Ok(json!({
                    "path": REGISTRY_RELATIVE_PATH,
                    "status": "degraded",
                    "error": error,
                    "hint": "fix the file with `planr agents check`; routing is disabled until it parses",
                }));
            }
            RegistryLoad::Loaded(registry) => registry,
        };
        let mut artifacts = Vec::new();
        for client in ["codex", "claude", "cursor"] {
            for (relative, expected) in self.agent_role_contents(client) {
                let Ok(actual) = std::fs::read_to_string(self.root.join(relative)) else {
                    continue;
                };
                let state = if !actual.contains(GENERATED_FROM_HEADER) {
                    "manual"
                } else if actual == expected {
                    "current"
                } else {
                    "drifted"
                };
                artifacts.push(json!({"path": relative, "state": state}));
            }
        }
        let drifted: Vec<&str> = artifacts
            .iter()
            .filter(|artifact| artifact["state"] == "drifted")
            .filter_map(|artifact| artifact["path"].as_str())
            .collect();
        Ok(json!({
            "path": REGISTRY_RELATIVE_PATH,
            "status": "ok",
            "profiles": registry.profiles.len(),
            "routes": registry.routes.len(),
            "default_route": registry.route_default.is_some(),
            "warnings": validation_warnings(&registry),
            "artifacts": artifacts,
            "drift_hint": (!drifted.is_empty()).then(|| format!(
                "rendered role file(s) out of date with the registry: {}; re-render with `planr install <client> --force`",
                drifted.join(", ")
            )),
        }))
    }

    pub(crate) fn item_route(&self, args: ItemRouteArgs) -> Result<()> {
        let (value, human) = if let Some(profile) = &args.set {
            self.item_route_set_value(&args.id, profile)?
        } else if args.clear {
            self.item_route_clear_value(&args.id)?
        } else {
            self.item_route_show_value(&args.id)?
        };
        self.emit(value, human)
    }

    pub(crate) fn item_route_show_value(&self, item_id: &str) -> Result<(Value, String)> {
        let facts = self.item_route_facts(item_id)?;
        let override_id = facts.route_override.clone();
        match load_registry(&self.root) {
            RegistryLoad::Missing => {
                let human = match &override_id {
                    Some(profile) => format!(
                        "override pinned to `{profile}` but no registry at {REGISTRY_RELATIVE_PATH}; declare the profile there to activate routing"
                    ),
                    None => {
                        format!("no route: no override and no registry at {REGISTRY_RELATIVE_PATH}")
                    }
                };
                Ok((
                    json!({"item": item_id, "override": override_id, "registry": "missing", "routing": null, "source": null}),
                    human,
                ))
            }
            RegistryLoad::Degraded { error } => Ok((
                json!({"item": item_id, "override": override_id, "registry": "degraded", "error": error, "routing": null, "source": null}),
                format!("no route: agent registry unusable: {error}"),
            )),
            RegistryLoad::Loaded(registry) => {
                let routing = resolve_route(&facts.as_routing_facts(), &registry);
                let source = routing.as_ref().map(|routing| {
                    if routing.matched_selector == "override" {
                        "override"
                    } else {
                        "policy"
                    }
                });
                // A pinned profile that has since left the registry is
                // advisory debt, not an error: policy keeps routing and
                // the hint says how to repair the pin.
                let dangling_hint = override_id.as_ref().and_then(|profile| {
                    (!registry.profiles.contains_key(profile)).then(|| {
                        format!(
                            "override `{profile}` is not in {REGISTRY_RELATIVE_PATH} (deleted or renamed?); policy routing applies — repair with `planr item route {item_id} --set <profile>` or `--clear`"
                        )
                    })
                });
                let human = match (&routing, source) {
                    (Some(routing), Some(source)) => format!(
                        "route [{source}]: {} ({} {}{}{}){}",
                        routing.profile,
                        routing.client,
                        routing.model,
                        routing
                            .effort
                            .map(|effort| format!(" effort={effort}"))
                            .unwrap_or_default(),
                        routing
                            .cost_tier
                            .map(|tier| format!(" tier={tier}"))
                            .unwrap_or_default(),
                        dangling_hint
                            .as_ref()
                            .map(|hint| format!("\n  hint: {hint}"))
                            .unwrap_or_default(),
                    ),
                    _ => format!(
                        "no route: nothing in {REGISTRY_RELATIVE_PATH} matches this item{}",
                        dangling_hint
                            .as_ref()
                            .map(|hint| format!("\n  hint: {hint}"))
                            .unwrap_or_default(),
                    ),
                };
                Ok((
                    json!({
                        "item": item_id,
                        "override": override_id,
                        "registry": "ok",
                        "routing": routing,
                        "source": source,
                        "hint": dangling_hint,
                    }),
                    human,
                ))
            }
        }
    }

    pub(crate) fn item_route_set_value(
        &self,
        item_id: &str,
        profile: &str,
    ) -> Result<(Value, String)> {
        self.get_item(item_id)?;
        let previous = self.item_route_override(item_id)?;
        // Offline edits stay possible: only a *loaded* registry can veto
        // the profile id; missing/degraded registries warn but store.
        let warning = match load_registry(&self.root) {
            RegistryLoad::Loaded(registry) => {
                if !registry.profiles.contains_key(profile) {
                    let known = registry.profiles.keys().cloned().collect::<Vec<_>>();
                    if known.is_empty() {
                        bail!(
                            "profile `{profile}` not found: {REGISTRY_RELATIVE_PATH} declares no profiles; add one under [profiles.{profile}] first"
                        );
                    }
                    bail!(
                        "profile `{profile}` is not declared in {REGISTRY_RELATIVE_PATH}; known profiles: {}",
                        known.join(", ")
                    );
                }
                None
            }
            RegistryLoad::Missing => Some(format!(
                "no registry at {REGISTRY_RELATIVE_PATH}; storing the override unvalidated"
            )),
            RegistryLoad::Degraded { error } => Some(format!(
                "agent registry unusable ({error}); storing the override unvalidated"
            )),
        };
        self.conn.execute(
            "UPDATE items SET route_override = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![profile, item_id],
        )?;
        self.record_event(
            "route_overridden",
            Some(item_id),
            json!({"profile": profile, "previous": previous, "validated": warning.is_none()}),
        )?;
        let routing = self.routing_value_for_item(item_id)?;
        let human = format!(
            "pinned {item_id} to profile `{profile}`{}",
            warning
                .as_ref()
                .map(|warning| format!("\n  warning: {warning}"))
                .unwrap_or_default(),
        );
        Ok((
            json!({
                "item": item_id,
                "override": profile,
                "previous": previous,
                "warning": warning,
                "routing": routing,
            }),
            human,
        ))
    }

    pub(crate) fn item_route_clear_value(&self, item_id: &str) -> Result<(Value, String)> {
        self.get_item(item_id)?;
        let previous = self.item_route_override(item_id)?;
        if previous.is_none() {
            return Ok((
                json!({"item": item_id, "cleared": false, "override": null}),
                format!("{item_id} has no route override; nothing to clear"),
            ));
        }
        self.conn.execute(
            "UPDATE items SET route_override = NULL, updated_at = datetime('now') WHERE id = ?1",
            params![item_id],
        )?;
        self.record_event(
            "route_override_cleared",
            Some(item_id),
            json!({"previous": previous}),
        )?;
        let routing = self.routing_value_for_item(item_id)?;
        Ok((
            json!({
                "item": item_id,
                "cleared": true,
                "previous": previous,
                "routing": routing,
            }),
            format!("cleared route override on {item_id}; policy routing applies"),
        ))
    }

    /// Role file contents for `planr install <client>`: when the registry
    /// loads and a role's route resolves to a profile whose `client`
    /// matches the install target, the static role text is re-rendered
    /// with that profile's model pin; in every other case (no registry,
    /// degraded registry, no route, no client-matching profile in the
    /// chain, unrenderable content) the shipped static text is used
    /// byte-identically. Worker roles follow the `work_type=code` route,
    /// reviewer roles the `work_type=review` route.
    pub(crate) fn agent_role_contents(&self, client: &str) -> Vec<(&'static str, String)> {
        let registry = match load_registry(&self.root) {
            RegistryLoad::Loaded(registry) => Some(registry),
            _ => None,
        };
        agent_roles(client)
            .iter()
            .map(|(relative, static_content)| {
                let rendered = registry
                    .as_ref()
                    .and_then(|registry| render_role(client, relative, static_content, registry));
                (
                    *relative,
                    rendered.unwrap_or_else(|| (*static_content).to_string()),
                )
            })
            .collect()
    }

    /// `planr prompt routing [--client ...]`: a paste-ready markdown block
    /// for the driver session — the route prioritization table from the
    /// registry, per-host dispatch guidance with the known traps (Codex
    /// fork_turns + session restart, Claude env preemption, Cursor silent
    /// overrides), and process-dispatch snippets for hosts without role
    /// files. Advisory output only: a missing or degraded registry still
    /// prints the host guidance with a pointer instead of failing.
    pub(crate) fn prompt_routing(&self, client: Option<ClientArg>) -> Result<()> {
        let client = client
            .map(|value| format!("{value:?}").to_lowercase())
            .unwrap_or_else(|| "all".to_string());
        let load = load_registry(&self.root);
        let (registry_status, registry) = match &load {
            RegistryLoad::Loaded(registry) => ("ok", Some(registry)),
            RegistryLoad::Missing => ("missing", None),
            RegistryLoad::Degraded { .. } => ("degraded", None),
        };
        let mut routes = Vec::new();
        let mut warnings = Vec::new();
        if let Some(registry) = registry {
            warnings = validation_warnings(registry);
            for route in &registry.routes {
                let selector = match (&route.selector.work_type, &route.selector.plan) {
                    (Some(work_type), _) => format!("work_type={work_type}"),
                    (None, Some(plan)) => format!("plan={plan}"),
                    (None, None) => "(empty match)".to_string(),
                };
                routes.push(route_table_row(
                    registry,
                    selector,
                    &route.profile,
                    &route.fallbacks,
                ));
            }
            if let Some(default) = &registry.route_default {
                routes.push(route_table_row(
                    registry,
                    "default".to_string(),
                    &default.profile,
                    &default.fallbacks,
                ));
            }
        }
        // The work_type=code route makes the snippets concrete; hosts
        // without one get neutral placeholders.
        let example = registry.and_then(|registry| {
            resolve_route(
                &RoutingFacts {
                    work_type: "code",
                    plan_id: None,
                    route_override: None,
                },
                registry,
            )
        });
        let example_model = example
            .as_ref()
            .map_or("<model>", |routing| routing.model)
            .to_string();
        let example_effort = example
            .as_ref()
            .and_then(|routing| routing.effort)
            .unwrap_or("<effort>")
            .to_string();
        let process_dispatch = vec![
            format!(
                "codex exec --model {example_model} -c model_reasoning_effort=\"{example_effort}\" \"<task>\""
            ),
            format!(
                "pi --provider <provider> --model {example_model} --thinking {example_effort} -p \"<task>\""
            ),
            format!(
                "opencode run --model \"<provider>/{example_model}\" \"<task>\"  # quote the provider/model pair"
            ),
        ];

        let mut prompt = String::from("## Model routing\n\n");
        match registry_status {
            "ok" if routes.is_empty() => prompt.push_str(
                "The registry declares no routes; add `[[routes]]` entries to .planr/agents.toml (see docs/MODEL_ROUTING.md).\n",
            ),
            "ok" => {
                prompt.push_str("Dispatch priority from .planr/agents.toml (first match wins; per-item `planr item route` pins beat all of it):\n\n");
                prompt.push_str("| match | profile | client | model | effort | tier | skill | fallbacks |\n");
                prompt.push_str("|---|---|---|---|---|---|---|---|\n");
                for row in &routes {
                    prompt.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                        row["match"].as_str().unwrap_or("-"),
                        row["profile"].as_str().unwrap_or("-"),
                        row["client"].as_str().unwrap_or("unknown profile"),
                        row["model"].as_str().unwrap_or("-"),
                        row["effort"].as_str().unwrap_or("-"),
                        row["cost_tier"].as_str().unwrap_or("-"),
                        row["skill"].as_str().unwrap_or("-"),
                        row["fallbacks"]
                            .as_array()
                            .filter(|list| !list.is_empty())
                            .map(|list| list
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", "))
                            .unwrap_or_else(|| "-".to_string()),
                    ));
                }
                if routes.iter().any(|row| row["skill"].is_string()) {
                    prompt.push_str(
                        "\nA `skill` names the paired skill for that profile: dispatch the worker with that skill (e.g. `Use $<skill> on item <id>`), not a hand-written prompt.\n",
                    );
                }
            }
            "missing" => prompt.push_str(
                "No .planr/agents.toml registry: routing is unset. Create one to declare profiles and routes (see docs/MODEL_ROUTING.md).\n",
            ),
            _ => prompt.push_str(
                "The .planr/agents.toml registry is unreadable; fix it with `planr agents check` before trusting any pins.\n",
            ),
        }
        for warning in &warnings {
            prompt.push_str(&format!("\nwarning: {warning}"));
        }
        if !warnings.is_empty() {
            prompt.push('\n');
        }
        let mut hosts = serde_json::Map::new();
        for (host, title, lines) in host_dispatch_sections() {
            if client != "all" && client != host {
                continue;
            }
            prompt.push_str(&format!("\n### {title}\n"));
            for line in lines {
                prompt.push_str(&format!("- {line}\n"));
            }
            hosts.insert(host.to_string(), json!(lines));
        }
        prompt.push_str("\n### Hosts without role files (process dispatch)\n");
        for line in &process_dispatch {
            prompt.push_str(&format!("- `{line}`\n"));
        }
        self.emit(
            json!({
                "mode": "routing",
                "client": client,
                "registry": registry_status,
                "routes": routes,
                "warnings": warnings,
                "hosts": hosts,
                "process_dispatch": process_dispatch,
                "prompt": prompt,
                "global_config_edited": false
            }),
            prompt.clone(),
        )
    }

    /// Owned routing inputs for one item, resolved from the graph: the
    /// item's work type, its plan id (via plan path), and any pin.
    fn item_route_facts(&self, item_id: &str) -> Result<ItemRouteInputs> {
        let item = self.get_item(item_id)?;
        let plan_id = item
            .plan_path
            .as_deref()
            .map(|path| self.plan_id_for_path(path))
            .transpose()?
            .flatten();
        let route_override = self.item_route_override(item_id)?;
        Ok(ItemRouteInputs {
            work_type: item.work_type.as_str().to_string(),
            plan_id,
            route_override,
        })
    }
}

fn render_role(
    client: &str,
    relative: &str,
    static_content: &str,
    registry: &AgentRegistry,
) -> Option<String> {
    let work_type = if relative.contains("worker") {
        "code"
    } else {
        "review"
    };
    let facts = RoutingFacts {
        work_type,
        plan_id: None,
        route_override: None,
    };
    let routing = resolve_route(&facts, registry)?;
    // A role file can only pin models its own host dispatches, so scan
    // the resolved chain for the first profile whose client matches the
    // install target — e.g. a review route pointing at a Cursor profile
    // must not write `fable-5` into a Codex TOML.
    let (profile_id, profile) = std::iter::once(routing.profile)
        .chain(routing.fallbacks.iter().copied())
        .find_map(|id| {
            let profile = registry.profiles.get(id)?;
            client_matches(client, &profile.client).then_some((id, profile))
        })?;
    // Bake the audit report into the worker's own definition so profile
    // reporting never depends on worker memory; reviewer roles carry
    // their explicit --reviewer instruction in the static body already.
    let evidence_note = (work_type == "code").then(|| {
        format!(
            "Report the profile this role runs on: pass `--profile {profile_id}` on every `planr done` and `planr log add`."
        )
    });
    let evidence_note = evidence_note.as_deref();
    match client {
        "codex" => render_codex_role(static_content, profile_id, profile, evidence_note),
        "claude" => render_claude_role(static_content, profile_id, profile, evidence_note),
        "cursor" => render_cursor_role(static_content, profile_id, profile, evidence_note),
        _ => None,
    }
}

/// One prioritization-table row. Unknown profiles keep their declared id
/// with null details so the table still names every route, matching the
/// `agents check` warnings instead of hiding the typo.
fn route_table_row(
    registry: &AgentRegistry,
    selector: String,
    profile_id: &str,
    fallbacks: &[String],
) -> Value {
    let profile = registry.profiles.get(profile_id);
    json!({
        "match": selector,
        "profile": profile_id,
        "client": profile.map(|profile| profile.client.clone()),
        "model": profile.map(|profile| profile.model.clone()),
        "effort": profile.and_then(|profile| profile.effort.clone()),
        "cost_tier": profile.and_then(|profile| profile.cost_tier.clone()),
        "skill": profile.and_then(|profile| profile.skill.clone()),
        "fallbacks": fallbacks,
    })
}

/// Host-native dispatch guidance with the traps from the July 2026
/// research (context ctx-bfa610fd): each pin has a host-side mechanism
/// that can silently defeat it, and the driver prompt must name them.
fn host_dispatch_sections() -> [(&'static str, &'static str, [&'static str; 3]); 3] {
    [
        (
            "codex",
            "Codex",
            [
                "Dispatch through the rendered role files (`.codex/agents/planr-worker.toml`, `planr-reviewer.toml`); re-render after registry edits with `planr install codex --force`.",
                "Subagent dispatch must use `fork_turns: \"none\"` — a full-history fork (`fork_turns = \"all\"`) intentionally drops `agent_type` and `model`, silently unpinning the role.",
                "Codex loads the agent role registry at session start; restart the session after re-rendering or the old pins stay live.",
            ],
        ),
        (
            "claude",
            "Claude Code",
            [
                "Dispatch the `planr-worker` subagent; the pin lives in `.claude/agents/planr-worker.md` frontmatter (`model:`, `effort:`).",
                "The `CLAUDE_CODE_SUBAGENT_MODEL` env var preempts every frontmatter pin (since v2.1.196 `inherit` means unset) — check it before trusting a pin.",
                "An org model allowlist falls back silently; smoke-test the child run's actual model once (docs/GOALS.md Cost Tiering).",
            ],
        ),
        (
            "cursor",
            "Cursor",
            [
                "Dispatch the `planr-worker` subagent; the pin lives in `.cursor/agents/planr-worker.md` frontmatter (`model:` only, no effort field).",
                "Plan mode, admin model policy, and Max Mode can override the pin silently; confirm the dispatched model in the child run metadata.",
                "Re-render pins after registry edits with `planr install cursor --force`.",
            ],
        ),
    ]
}

/// Install targets vs registry `client` vocabulary: the install command
/// says `claude`, the registry documents `claude-code`.
fn client_matches(install_target: &str, profile_client: &str) -> bool {
    match install_target {
        "claude" => matches!(profile_client, "claude" | "claude-code"),
        other => profile_client == other,
    }
}

/// Registry client vocabulary comparison for the run audit: `claude`
/// and `claude-code` are the same host.
fn client_vocab_matches(declared: &str, observed: &str) -> bool {
    let canonical = |value: &str| {
        if value == "claude" {
            "claude-code"
        } else {
            value
        }
        .to_string()
    };
    canonical(declared) == canonical(observed)
}

struct ItemRouteInputs {
    work_type: String,
    plan_id: Option<String>,
    route_override: Option<String>,
}

impl ItemRouteInputs {
    fn as_routing_facts(&self) -> RoutingFacts<'_> {
        RoutingFacts {
            work_type: &self.work_type,
            plan_id: self.plan_id.as_deref(),
            route_override: self.route_override.as_deref(),
        }
    }
}
