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
use crate::rolefiles::agent_roles;
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
                    let mut warnings = validation_warnings(&registry);
                    warnings.extend(self.skill_existence_warnings(&registry));
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
            // Discoverability alias: `agents routing` is where people
            // guess the dispatch block lives (dogfood finding F6).
            AgentsCommand::Routing(args) => self.prompt_routing(args.client),
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

    /// Advisory check that pinned profile skills actually exist
    /// somewhere a host could load them (project and home skill
    /// directories). Skills stay passthrough vocabulary (ADR-102) — a
    /// missing one is a warning, never an error: the dogfood run showed
    /// a worker silently operating without its pinned skill.
    fn skill_existence_warnings(&self, registry: &AgentRegistry) -> Vec<String> {
        let home = std::env::var("HOME").map(std::path::PathBuf::from).ok();
        let mut roots = vec![self.root.clone()];
        if let Some(home) = home {
            roots.push(home);
        }
        let dirs = [
            ".cursor/skills",
            ".claude/skills",
            ".agents/skills",
            ".codex/skills",
        ];
        let mut warnings = Vec::new();
        for (id, profile) in &registry.profiles {
            let Some(skill) = profile.skill.as_deref() else {
                continue;
            };
            let found = roots.iter().any(|root| {
                dirs.iter()
                    .any(|dir| root.join(dir).join(skill).join("SKILL.md").exists())
            });
            if !found {
                warnings.push(format!(
                    "profile `{id}` pins skill `{skill}` but no SKILL.md was found under the project or home skill directories (.cursor/.claude/.agents/.codex); workers will run without it"
                ));
            }
        }
        warnings
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
    /// allowlists, unavailable role models), so the recorded profile is the
    /// only trustworthy signal. A mismatch emits exactly one
    /// `route_mismatch_observed` event — it never blocks logging, review,
    /// or closure. No declared route (no registry, nothing resolves)
    /// means no comparison and no event.
    pub(crate) fn observe_route_compliance(
        &self,
        item_id: &str,
        run_id: &str,
        actual_profile: &str,
        log_kind: &str,
    ) -> Result<()> {
        let Some(routing) = self.routing_value_for_item(item_id)? else {
            return Ok(());
        };
        let Some(declared) = routing["profile"].as_str() else {
            return Ok(());
        };
        if declared != actual_profile {
            // log_kind lets audit consumers discount the legitimate
            // case: a driver adding a verification log to a routed item
            // runs on the driver profile by design.
            self.record_event(
                "route_mismatch_observed",
                Some(item_id),
                json!({
                    "declared_profile": declared,
                    "actual_profile": actual_profile,
                    "run_id": run_id,
                    "log_kind": log_kind,
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
    pub(crate) fn observe_client_compliance(
        &self,
        item_id: &str,
        run_id: &str,
        log_kind: &str,
    ) -> Result<()> {
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
                    "log_kind": log_kind,
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
        let any_observed = runs
            .iter()
            .any(|run| run.get("route_observation").is_some());
        if declared.is_none() && !any_profiled && !any_observed {
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
    /// warnings. Host artifacts are owned and verified by external tools.
    pub(crate) fn registry_doctor_value(&self) -> Result<Value> {
        let registry = match load_registry(&self.root) {
            RegistryLoad::Missing => {
                return Ok(json!({
                    "path": REGISTRY_RELATIVE_PATH,
                    "status": "absent",
                    "hint": "no agent registry; create .planr/agents.toml to declare model routing (https://planr.so/docs/plugins)",
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
        Ok(json!({
            "path": REGISTRY_RELATIVE_PATH,
            "status": "ok",
            "profiles": registry.profiles.len(),
            "routes": registry.routes.len(),
            "default_route": registry.route_default.is_some(),
            "warnings": validation_warnings(&registry),
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

    /// Static workflow role files for `planr install <client>`. Model and
    /// effort pins belong to external routing tools, never to Planr Core.
    pub(crate) fn agent_role_contents(&self, client: &str) -> Vec<(&'static str, String)> {
        agent_roles(client)
            .iter()
            .map(|(relative, content)| (*relative, (*content).to_string()))
            .collect()
    }

    /// `planr prompt routing [--client ...]`: a paste-ready markdown block
    /// for the driver session — the route prioritization table from the
    /// registry plus provider-neutral dispatch and evidence guidance.
    /// Advisory output only: a missing or degraded registry still
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
        let process_dispatch = match client.as_str() {
            "all" => vec![
                format!(
                    "pi --provider <provider> --model {example_model} --thinking {example_effort} -p \"<task>\""
                ),
                format!(
                    "opencode run --model \"<provider>/{example_model}\" \"<task>\"  # quote the provider/model pair"
                ),
            ],
            "pi" => vec![
                "pi --approve --model <provider/model> --thinking <level> -p \"Use /skill:planr-work on item <item-id>. Stop after requesting review.\"".to_string(),
            ],
            _ => Vec::new(),
        };

        let mut prompt = String::from("## Model routing\n\n");
        match registry_status {
            "ok" if routes.is_empty() => prompt.push_str(
                "The registry declares no routes; add `[[routes]]` entries to .planr/agents.toml (see https://planr.so/docs/plugins).\n",
            ),
            "ok" => {
                prompt.push_str("Dispatch priority from .planr/agents.toml (first match wins; per-item `planr item route` pins beat all of it):\n\n");
                prompt.push_str("| match | profile | client | agent_type | model | effort | tier | skill | fallbacks |\n");
                prompt.push_str("|---|---|---|---|---|---|---|---|---|\n");
                for row in &routes {
                    prompt.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                        row["match"].as_str().unwrap_or("-"),
                        row["profile"].as_str().unwrap_or("-"),
                        row["client"].as_str().unwrap_or("unknown profile"),
                        row["agent_type"].as_str().unwrap_or("-"),
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
                "No .planr/agents.toml registry: routing is unset. Create one to declare profiles and routes (see https://planr.so/docs/plugins).\n",
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
        let generic_guidance = [
            "Dispatch the selected repository profile through the active host.",
            "Treat declared model and effort as requested until effective run evidence confirms them.",
            "Apply host-specific roles and skills only through externally generated repository declarations.",
        ];
        prompt.push_str("\n### Host dispatch\n");
        for line in generic_guidance {
            prompt.push_str(&format!("- {line}\n"));
        }
        hosts.insert("generic".to_string(), json!(generic_guidance));
        if client == "grok" {
            let grok_guidance = [
                "Use repository-native `.grok/skills` and `.grok/agents` installed by `planr install grok`.",
                "Grok V1 installs no Planr hooks; run `planr prime` manually when fresh state is needed.",
                "For headless runs use `grok --no-auto-update -p \"<task>\" --output-format json`.",
            ];
            prompt.push_str("\n### Grok Build\n");
            for line in grok_guidance {
                prompt.push_str(&format!("- {line}\n"));
            }
            hosts.insert("grok".to_string(), json!(grok_guidance));
        }
        if client == "pi" {
            let pi_guidance = [
                "Use `/skill:planr` to load the repository-native router installed by `planr install pi`; Pi must trust the project before `.pi` resources load.",
                "When optional `pi-subagents` is installed, dispatch `.pi/agents/planr-worker.md` and `planr-reviewer.md`; otherwise use separate sequential Pi processes and record single-agent review honestly.",
                "Pi V1 installs no Planr MCP adapter or hooks; run `planr prime` manually when fresh state is needed.",
                "For intentional headless project-resource loading use `pi --approve -p \"<prompt>\"`; never write Pi trust or global settings on the user's behalf.",
            ];
            prompt.push_str("\n### Pi\n");
            for line in pi_guidance {
                prompt.push_str(&format!("- {line}\n"));
            }
            hosts.insert("pi".to_string(), json!(pi_guidance));
        }
        if !process_dispatch.is_empty() {
            prompt.push_str("\n### Hosts without role files (process dispatch)\n");
            for line in &process_dispatch {
                prompt.push_str(&format!("- `{line}`\n"));
            }
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
        "agent_type": profile.and_then(|profile| profile.agent_type.clone()),
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
