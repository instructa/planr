//! CLI and MCP surface for the agent profile registry and per-item
//! routing. The `*_value` functions own the JSON shapes; the CLI adds
//! human rendering on top and MCP tools return the same values, so both
//! surfaces stay contract-identical by construction.

use super::App;
use crate::agents::{
    AgentRegistry, REGISTRY_RELATIVE_PATH, RegistryLoad, RoutingFacts, load_registry,
    resolve_route, validation_warnings,
};
use crate::cli::{AgentsCommand, ItemRouteArgs};
use crate::rolefiles::{agent_roles, render_claude_role, render_codex_role, render_cursor_role};
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
                        "\n  {id}: {} {}{}{}",
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
    match client {
        "codex" => render_codex_role(static_content, profile_id, profile),
        "claude" => render_claude_role(static_content, profile_id, profile),
        "cursor" => render_cursor_role(static_content, profile_id, profile),
        _ => None,
    }
}

/// Install targets vs registry `client` vocabulary: the install command
/// says `claude`, the registry documents `claude-code`.
fn client_matches(install_target: &str, profile_client: &str) -> bool {
    match install_target {
        "claude" => matches!(profile_client, "claude" | "claude-code"),
        other => profile_client == other,
    }
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
