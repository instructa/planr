//! Agent profile registry: `.planr/agents.toml` declares named agent
//! profiles (host client + model + effort + cost tier) and advisory routes
//! from work selectors to profiles. Planr never calls model providers;
//! this module only parses configuration so other layers can recommend a
//! profile per item. A missing registry means "no routing", and a
//! malformed one degrades with a diagnostic instead of failing commands.

use crate::secrets::looks_secret_like;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const REGISTRY_RELATIVE_PATH: &str = ".planr/agents.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfile {
    /// Host client that dispatches this profile: codex, claude-code,
    /// cursor, grok, pi, or generic-mcp. Free-form so new hosts need no release.
    pub client: String,
    /// Model alias or full id, passed through verbatim — Planr does not
    /// validate ids against provider catalogs.
    pub model: String,
    /// Optional host role selector, passed through without host-specific validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// premium | standard | budget (advisory vocabulary, not enforced).
    #[serde(default)]
    pub cost_tier: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Skill this profile pairs with (e.g. `planr-work`,
    /// `frontend-design`): dispatch the profile *with* this skill.
    /// Passthrough vocabulary — never validated against installed skills.
    #[serde(default)]
    pub skill: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSelector {
    #[serde(default)]
    pub work_type: Option<String>,
    /// Plan id (e.g. `pln-1234abcd`) the item belongs to.
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    #[serde(rename = "match")]
    pub selector: RouteSelector,
    pub profile: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultRoute {
    pub profile: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRegistry {
    #[serde(default)]
    pub profiles: BTreeMap<String, AgentProfile>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub route_default: Option<DefaultRoute>,
}

#[derive(Debug)]
pub enum RegistryLoad {
    /// No `.planr/agents.toml`: routing is simply absent, never an error.
    Missing,
    /// The file exists but cannot be used; `error` carries the TOML
    /// parser's line/column context. Callers must keep working.
    Degraded {
        error: String,
    },
    Loaded(AgentRegistry),
}

pub fn registry_path(root: &Path) -> PathBuf {
    root.join(REGISTRY_RELATIVE_PATH)
}

pub fn load_registry(root: &Path) -> RegistryLoad {
    let text = match std::fs::read_to_string(registry_path(root)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return RegistryLoad::Missing,
        Err(err) => {
            return RegistryLoad::Degraded {
                error: err.to_string(),
            };
        }
    };
    parse_registry(&text)
}

pub fn parse_registry(text: &str) -> RegistryLoad {
    match toml::from_str::<AgentRegistry>(text) {
        Ok(registry) => RegistryLoad::Loaded(registry),
        Err(err) => RegistryLoad::Degraded {
            error: err.to_string(),
        },
    }
}

/// Problems a parseable registry can still have. `agents check` surfaces
/// them without making the file unreadable.
pub fn validation_warnings(registry: &AgentRegistry) -> Vec<String> {
    let mut warnings = Vec::new();
    for (route_label, profile, fallbacks) in route_targets(registry) {
        for referenced in std::iter::once(profile).chain(fallbacks.iter().map(String::as_str)) {
            if !registry.profiles.contains_key(referenced) {
                warnings.push(format!(
                    "{route_label} references unknown profile `{referenced}`"
                ));
            }
        }
    }
    for (index, route) in registry.routes.iter().enumerate() {
        let label = route_label(index);
        match (&route.selector.work_type, &route.selector.plan) {
            (None, None) => warnings.push(format!(
                "{label} has an empty match; set `work_type` or `plan` or it will never route"
            )),
            (Some(_), Some(_)) => warnings.push(format!(
                "{label} sets both `work_type` and `plan`; only one selector per route is supported and `work_type` wins"
            )),
            _ => {}
        }
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, route) in registry.routes.iter().enumerate() {
        let Some(key) = selector_key(&route.selector) else {
            continue;
        };
        if let Some(first) = seen.get(&key) {
            warnings.push(format!(
                "{} duplicates the selector of {}; the first route wins",
                route_label(index),
                route_label(*first)
            ));
        } else {
            seen.insert(key, index);
        }
    }
    for (id, profile) in &registry.profiles {
        for field in profile_strings(profile) {
            if looks_secret_like(field) {
                warnings.push(format!(
                    "profile `{id}` contains a secret-like value; the registry must hold configuration only, never credentials"
                ));
                break;
            }
        }
    }
    warnings
}

fn route_targets(registry: &AgentRegistry) -> Vec<(String, &str, &Vec<String>)> {
    let mut targets: Vec<(String, &str, &Vec<String>)> = registry
        .routes
        .iter()
        .enumerate()
        .map(|(index, route)| (route_label(index), route.profile.as_str(), &route.fallbacks))
        .collect();
    if let Some(default) = &registry.route_default {
        targets.push((
            "[route_default]".to_string(),
            default.profile.as_str(),
            &default.fallbacks,
        ));
    }
    targets
}

fn route_label(index: usize) -> String {
    format!("[[routes]] #{}", index + 1)
}

fn selector_key(selector: &RouteSelector) -> Option<String> {
    match (&selector.work_type, &selector.plan) {
        (Some(work_type), _) => Some(format!("work_type={work_type}")),
        (None, Some(plan)) => Some(format!("plan={plan}")),
        (None, None) => None,
    }
}

fn profile_strings(profile: &AgentProfile) -> impl Iterator<Item = &str> {
    [
        Some(profile.client.as_str()),
        Some(profile.model.as_str()),
        profile.agent_type.as_deref(),
        profile.effort.as_deref(),
        profile.cost_tier.as_deref(),
        profile.notes.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(profile.capabilities.iter().map(String::as_str))
}

/// The facts about a map item that routing may select on. The caller
/// resolves `plan_id` from the item's plan path; core stays free of
/// storage and host concerns.
#[derive(Debug, Default)]
pub struct RoutingFacts<'a> {
    pub work_type: &'a str,
    pub plan_id: Option<&'a str>,
    /// Per-item pinned profile id (`items.route_override`). When it names
    /// a known profile it beats every policy route; an unknown id falls
    /// through to policy so a stale pin never silences routing.
    pub route_override: Option<&'a str>,
}

/// An advisory routing recommendation: the strongest claim Planr makes
/// is "dispatch this profile, fall back in this order". Hosts remain the
/// dispatch authority.
#[derive(Debug, Clone, Serialize)]
pub struct Routing<'a> {
    pub profile: &'a str,
    pub client: &'a str,
    pub model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub cost_tier: Option<&'a str>,
    /// Paired skill from the profile; omitted from serialized routing
    /// blocks when absent so no-skill registries stay byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<&'a str>,
    /// Known-profile fallback ids in declared order; unknown ids are
    /// skipped rather than surfaced to dispatchers.
    pub fallbacks: Vec<&'a str>,
    /// Which selector won: `override`, `work_type=<v>`, `plan=<v>`, or
    /// `default`.
    pub matched_selector: String,
}

/// Resolves an item to a profile recommendation. Precedence is
/// override > work_type > plan > default; a route on a double selector
/// keeps its `work_type` meaning (the stricter documented behavior). A
/// route whose entire chain references unknown profiles is skipped so a
/// typo never swallows lower-precedence routes — the same rule makes an
/// override naming a deleted profile fall through to policy. Returns
/// None when nothing resolves.
pub fn resolve_route<'a>(
    facts: &RoutingFacts<'_>,
    registry: &'a AgentRegistry,
) -> Option<Routing<'a>> {
    if let Some(override_id) = facts.route_override {
        if let Some(routing) = routing_for_chain(registry, "override".to_string(), override_id, &[])
        {
            return Some(routing);
        }
    }
    let work_type_route = registry.routes.iter().find_map(|route| {
        let selector = route.selector.work_type.as_deref()?;
        (selector == facts.work_type).then_some((
            format!("work_type={selector}"),
            route.profile.as_str(),
            &route.fallbacks,
        ))
    });
    let plan_route = registry.routes.iter().find_map(|route| {
        if route.selector.work_type.is_some() {
            return None;
        }
        let selector = route.selector.plan.as_deref()?;
        (Some(selector) == facts.plan_id).then_some((
            format!("plan={selector}"),
            route.profile.as_str(),
            &route.fallbacks,
        ))
    });
    let default_route = registry.route_default.as_ref().map(|default| {
        (
            "default".to_string(),
            default.profile.as_str(),
            &default.fallbacks,
        )
    });
    [work_type_route, plan_route, default_route]
        .into_iter()
        .flatten()
        .find_map(|(matched_selector, profile, fallbacks)| {
            routing_for_chain(registry, matched_selector, profile, fallbacks)
        })
}

/// Builds the routing from the first known profile in the chain; the
/// remaining known ids become the fallback list.
fn routing_for_chain<'a>(
    registry: &'a AgentRegistry,
    matched_selector: String,
    profile: &str,
    fallbacks: &[String],
) -> Option<Routing<'a>> {
    let mut known = std::iter::once(profile)
        .chain(fallbacks.iter().map(String::as_str))
        .filter_map(|id| registry.profiles.get_key_value(id));
    let (primary_id, primary) = known.next()?;
    Some(Routing {
        profile: primary_id,
        client: &primary.client,
        model: &primary.model,
        agent_type: primary.agent_type.as_deref(),
        effort: primary.effort.as_deref(),
        cost_tier: primary.cost_tier.as_deref(),
        skill: primary.skill.as_deref(),
        fallbacks: known.map(|(id, _)| id.as_str()).collect(),
        matched_selector,
    })
}

#[cfg(test)]
mod tests;
