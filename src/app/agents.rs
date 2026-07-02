//! CLI surface for the agent profile registry and per-item routing.

use super::App;
use crate::agents::{
    REGISTRY_RELATIVE_PATH, RegistryLoad, RoutingFacts, load_registry, resolve_route,
    validation_warnings,
};
use crate::cli::AgentsCommand;
use anyhow::{Result, bail};
use serde_json::{Value, json};

impl App {
    pub(crate) fn agents(&self, command: AgentsCommand) -> Result<()> {
        match command {
            AgentsCommand::List(_) => match load_registry(&self.root) {
                RegistryLoad::Missing => self.emit(
                    json!({"registry": null, "reason": "missing", "path": REGISTRY_RELATIVE_PATH}),
                    format!(
                        "no agent registry; declare profiles and routes in {REGISTRY_RELATIVE_PATH} to get routing recommendations in pick packets"
                    ),
                ),
                RegistryLoad::Degraded { error } => self.emit(
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
                    self.emit(json!({"registry": registry, "warnings": warnings}), human)
                }
            },
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

    /// The advisory routing block for a pick packet, or None when no
    /// registry exists, it is degraded, or nothing resolves — the packet
    /// simply omits `routing` in all of those cases.
    pub(crate) fn routing_value_for_item(&self, item_id: &str) -> Result<Option<Value>> {
        let RegistryLoad::Loaded(registry) = load_registry(&self.root) else {
            return Ok(None);
        };
        let item = self.get_item(item_id)?;
        let plan_id = item
            .plan_path
            .as_deref()
            .map(|path| self.plan_id_for_path(path))
            .transpose()?
            .flatten();
        let facts = RoutingFacts {
            work_type: item.work_type.as_str(),
            plan_id: plan_id.as_deref(),
        };
        Ok(resolve_route(&facts, &registry).map(|routing| json!(routing)))
    }
}
