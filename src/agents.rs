//! Agent profile registry: `.planr/agents.toml` declares named agent
//! profiles (host client + model + effort + cost tier) and advisory routes
//! from work selectors to profiles. Planr never calls model providers;
//! this module only parses configuration so other layers can recommend a
//! profile per item. A missing registry means "no routing", and a
//! malformed one degrades with a diagnostic instead of failing commands.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const REGISTRY_RELATIVE_PATH: &str = ".planr/agents.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfile {
    /// Host client that dispatches this profile: codex, claude-code,
    /// cursor, or generic-mcp. Free-form so new hosts need no release.
    pub client: String,
    /// Model alias or full id, passed through verbatim — Planr does not
    /// validate ids against provider catalogs.
    pub model: String,
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

/// Non-fatal problems a parseable registry can still have. Warnings never
/// block parsing, picking, or resolution — `agents check` surfaces them.
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
        if route.selector.work_type.as_deref() == Some("review")
            && first_known_tier(registry, &route.profile, &route.fallbacks) == Some("budget")
        {
            warnings.push(format!(
                "{label} routes review work to a budget-tier profile; verdicts should stay on the strongest tier"
            ));
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

fn first_known_tier<'a>(
    registry: &'a AgentRegistry,
    profile: &str,
    fallbacks: &[String],
) -> Option<&'a str> {
    std::iter::once(profile)
        .chain(fallbacks.iter().map(String::as_str))
        .find_map(|id| registry.profiles.get(id))
        .and_then(|profile| profile.cost_tier.as_deref())
}

fn profile_strings(profile: &AgentProfile) -> impl Iterator<Item = &str> {
    [
        Some(profile.client.as_str()),
        Some(profile.model.as_str()),
        profile.effort.as_deref(),
        profile.cost_tier.as_deref(),
        profile.notes.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(profile.capabilities.iter().map(String::as_str))
}

fn looks_secret_like(text: &str) -> bool {
    ["sk-", "ghp_", "BEGIN PRIVATE KEY", "AKIA"]
        .iter()
        .any(|pattern| text.contains(pattern))
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
        effort: primary.effort.as_deref(),
        cost_tier: primary.cost_tier.as_deref(),
        skill: primary.skill.as_deref(),
        fallbacks: known.map(|(id, _)| id.as_str()).collect(),
        matched_selector,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
[profiles.codex-implementer]
client = "codex"
model = "gpt-5.5"
effort = "medium"
cost_tier = "standard"
capabilities = ["code", "steerable"]
notes = "Primary implementation model."

[profiles.cursor-fable-driver]
client = "cursor"
model = "fable-5"
effort = "high"
cost_tier = "premium"

[[routes]]
match = { work_type = "code" }
profile = "codex-implementer"
fallbacks = ["cursor-fable-driver"]

[route_default]
profile = "codex-implementer"
"#;

    #[test]
    fn parses_profiles_routes_and_default() {
        let RegistryLoad::Loaded(registry) = parse_registry(VALID) else {
            panic!("expected loaded registry");
        };
        assert_eq!(registry.profiles.len(), 2);
        let implementer = &registry.profiles["codex-implementer"];
        assert_eq!(implementer.client, "codex");
        assert_eq!(implementer.model, "gpt-5.5");
        assert_eq!(implementer.effort.as_deref(), Some("medium"));
        assert_eq!(implementer.cost_tier.as_deref(), Some("standard"));
        assert_eq!(implementer.capabilities, ["code", "steerable"]);
        assert_eq!(registry.routes.len(), 1);
        assert_eq!(
            registry.routes[0].selector.work_type.as_deref(),
            Some("code")
        );
        assert_eq!(registry.routes[0].fallbacks, ["cursor-fable-driver"]);
        assert_eq!(
            registry.route_default.as_ref().map(|d| d.profile.as_str()),
            Some("codex-implementer")
        );
        assert!(validation_warnings(&registry).is_empty());
    }

    #[test]
    fn missing_file_is_missing_not_error() {
        let dir = std::env::temp_dir().join("planr-agents-missing-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(load_registry(&dir), RegistryLoad::Missing));
    }

    #[test]
    fn malformed_toml_degrades_with_line_context() {
        let RegistryLoad::Degraded { error } = parse_registry("profiles = [broken") else {
            panic!("expected degraded registry");
        };
        assert!(error.contains("line 1"), "no line context in: {error}");
    }

    #[test]
    fn unknown_field_degrades_with_typo_context() {
        let text = "[profiles.a]\nclient = \"codex\"\nmodel = \"gpt-5.5\"\nefort = \"high\"\n";
        let RegistryLoad::Degraded { error } = parse_registry(text) else {
            panic!("expected degraded registry");
        };
        assert!(error.contains("efort"), "typo not named in: {error}");
    }

    #[test]
    fn missing_required_profile_field_degrades() {
        let text = "[profiles.a]\nclient = \"codex\"\n";
        let RegistryLoad::Degraded { error } = parse_registry(text) else {
            panic!("expected degraded registry");
        };
        assert!(error.contains("model"), "missing field not named: {error}");
    }

    #[test]
    fn warns_on_unknown_profile_references() {
        let text = r#"
[profiles.a]
client = "codex"
model = "gpt-5.5"

[[routes]]
match = { work_type = "code" }
profile = "ghost"
fallbacks = ["a", "phantom"]

[route_default]
profile = "spirit"
"#;
        let RegistryLoad::Loaded(registry) = parse_registry(text) else {
            panic!("expected loaded registry");
        };
        let warnings = validation_warnings(&registry);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("`ghost`"));
        assert!(warnings[1].contains("`phantom`"));
        assert!(warnings[2].contains("`spirit`"));
    }

    #[test]
    fn warns_on_empty_and_double_selectors_and_duplicates() {
        let text = r#"
[profiles.a]
client = "codex"
model = "gpt-5.5"

[[routes]]
match = {}
profile = "a"

[[routes]]
match = { work_type = "code", plan = "pln-1" }
profile = "a"

[[routes]]
match = { work_type = "code" }
profile = "a"
"#;
        let RegistryLoad::Loaded(registry) = parse_registry(text) else {
            panic!("expected loaded registry");
        };
        let warnings = validation_warnings(&registry);
        assert!(warnings.iter().any(|w| w.contains("empty match")));
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("both `work_type` and `plan`"))
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("duplicates the selector"))
        );
    }

    #[test]
    fn warns_when_review_routes_to_budget_tier() {
        let text = r#"
[profiles.cheap]
client = "cursor"
model = "composer-2.5"
cost_tier = "budget"

[[routes]]
match = { work_type = "review" }
profile = "cheap"
"#;
        let RegistryLoad::Loaded(registry) = parse_registry(text) else {
            panic!("expected loaded registry");
        };
        let warnings = validation_warnings(&registry);
        assert!(
            warnings.iter().any(|w| w.contains("budget-tier")),
            "{warnings:?}"
        );
    }

    #[test]
    fn warns_on_secret_like_profile_values() {
        let text =
            "[profiles.a]\nclient = \"codex\"\nmodel = \"gpt-5.5\"\nnotes = \"key sk-abc123\"\n";
        let RegistryLoad::Loaded(registry) = parse_registry(text) else {
            panic!("expected loaded registry");
        };
        let warnings = validation_warnings(&registry);
        assert!(
            warnings.iter().any(|w| w.contains("secret-like")),
            "{warnings:?}"
        );
    }

    fn registry(text: &str) -> AgentRegistry {
        match parse_registry(text) {
            RegistryLoad::Loaded(registry) => registry,
            other => panic!("expected loaded registry, got {other:?}"),
        }
    }

    const ROUTING: &str = r#"
[profiles.implementer]
client = "codex"
model = "gpt-5.5"
effort = "medium"
cost_tier = "standard"

[profiles.driver]
client = "cursor"
model = "fable-5"
effort = "high"
cost_tier = "premium"

[profiles.browser]
client = "cursor"
model = "composer-2.5"
cost_tier = "budget"

[[routes]]
match = { work_type = "code" }
profile = "implementer"
fallbacks = ["driver"]

[[routes]]
match = { work_type = "review" }
profile = "driver"

[[routes]]
match = { plan = "pln-web" }
profile = "browser"

[route_default]
profile = "implementer"
fallbacks = ["driver"]
"#;

    fn facts<'a>(work_type: &'a str, plan_id: Option<&'a str>) -> RoutingFacts<'a> {
        RoutingFacts {
            work_type,
            plan_id,
            route_override: None,
        }
    }

    fn facts_with_override<'a>(work_type: &'a str, route_override: &'a str) -> RoutingFacts<'a> {
        RoutingFacts {
            work_type,
            plan_id: None,
            route_override: Some(route_override),
        }
    }

    #[test]
    fn resolves_work_type_route_with_fallbacks() {
        let registry = registry(ROUTING);
        let routing = resolve_route(&facts("code", None), &registry).unwrap();
        assert_eq!(routing.profile, "implementer");
        assert_eq!(routing.client, "codex");
        assert_eq!(routing.model, "gpt-5.5");
        assert_eq!(routing.effort, Some("medium"));
        assert_eq!(routing.cost_tier, Some("standard"));
        assert_eq!(routing.fallbacks, ["driver"]);
        assert_eq!(routing.matched_selector, "work_type=code");
    }

    #[test]
    fn work_type_route_beats_plan_route() {
        let registry = registry(ROUTING);
        let routing = resolve_route(&facts("review", Some("pln-web")), &registry).unwrap();
        assert_eq!(routing.profile, "driver");
        assert_eq!(routing.matched_selector, "work_type=review");
    }

    #[test]
    fn plan_route_beats_default() {
        let registry = registry(ROUTING);
        let routing = resolve_route(&facts("docs", Some("pln-web")), &registry).unwrap();
        assert_eq!(routing.profile, "browser");
        assert_eq!(routing.matched_selector, "plan=pln-web");
    }

    #[test]
    fn default_route_catches_unmatched_items() {
        let registry = registry(ROUTING);
        let routing = resolve_route(&facts("docs", None), &registry).unwrap();
        assert_eq!(routing.profile, "implementer");
        assert_eq!(routing.fallbacks, ["driver"]);
        assert_eq!(routing.matched_selector, "default");
    }

    #[test]
    fn skill_pairs_through_resolution_and_skips_serialization_when_absent() {
        let pool = registry(
            "[profiles.designer]\nclient = \"claude-code\"\nmodel = \"opus\"\nskill = \"frontend-design\"\n\n[[routes]]\nmatch = { work_type = \"frontend\" }\nprofile = \"designer\"\n",
        );
        let routing = resolve_route(&facts("frontend", None), &pool).unwrap();
        assert_eq!(routing.skill, Some("frontend-design"));
        let serialized = serde_json::to_value(&routing).unwrap();
        assert_eq!(serialized["skill"], "frontend-design");

        // No-skill profiles omit the key entirely (byte-identical blocks).
        let plain = registry(ROUTING);
        let routing = resolve_route(&facts("code", None), &plain).unwrap();
        assert_eq!(routing.skill, None);
        let serialized = serde_json::to_value(&routing).unwrap();
        assert!(!serialized.as_object().unwrap().contains_key("skill"));
    }

    #[test]
    fn no_routes_and_no_default_resolves_none() {
        let registry = registry("[profiles.a]\nclient = \"codex\"\nmodel = \"gpt-5.5\"\n");
        assert!(resolve_route(&facts("code", None), &registry).is_none());
    }

    #[test]
    fn unknown_primary_promotes_first_known_fallback() {
        let text = r#"
[profiles.driver]
client = "cursor"
model = "fable-5"

[[routes]]
match = { work_type = "code" }
profile = "ghost"
fallbacks = ["phantom", "driver"]
"#;
        let registry = registry(text);
        let routing = resolve_route(&facts("code", None), &registry).unwrap();
        assert_eq!(routing.profile, "driver");
        assert!(routing.fallbacks.is_empty());
        assert_eq!(routing.matched_selector, "work_type=code");
    }

    #[test]
    fn fully_unknown_chain_falls_through_to_next_precedence() {
        let text = r#"
[profiles.driver]
client = "cursor"
model = "fable-5"

[[routes]]
match = { work_type = "code" }
profile = "ghost"

[route_default]
profile = "driver"
"#;
        let registry = registry(text);
        let routing = resolve_route(&facts("code", None), &registry).unwrap();
        assert_eq!(routing.profile, "driver");
        assert_eq!(routing.matched_selector, "default");
    }

    #[test]
    fn first_declared_route_wins_within_a_level() {
        let text = r#"
[profiles.a]
client = "codex"
model = "gpt-5.5"

[profiles.b]
client = "cursor"
model = "fable-5"

[[routes]]
match = { work_type = "code" }
profile = "a"

[[routes]]
match = { work_type = "code" }
profile = "b"
"#;
        let registry = registry(text);
        let routing = resolve_route(&facts("code", None), &registry).unwrap();
        assert_eq!(routing.profile, "a");
    }

    #[test]
    fn double_selector_route_matches_on_work_type_only() {
        let text = r#"
[profiles.a]
client = "codex"
model = "gpt-5.5"

[[routes]]
match = { work_type = "code", plan = "pln-x" }
profile = "a"
"#;
        let registry = registry(text);
        // Matches on work_type even for a different plan.
        let routing = resolve_route(&facts("code", Some("pln-other")), &registry).unwrap();
        assert_eq!(routing.matched_selector, "work_type=code");
        // Never matches as a plan selector.
        assert!(resolve_route(&facts("docs", Some("pln-x")), &registry).is_none());
    }

    #[test]
    fn override_beats_work_type_route() {
        let registry = registry(ROUTING);
        let routing = resolve_route(&facts_with_override("code", "driver"), &registry).unwrap();
        assert_eq!(routing.profile, "driver");
        assert_eq!(routing.client, "cursor");
        assert_eq!(routing.model, "fable-5");
        assert!(routing.fallbacks.is_empty());
        assert_eq!(routing.matched_selector, "override");
    }

    #[test]
    fn unknown_override_falls_through_to_policy() {
        let registry = registry(ROUTING);
        let routing = resolve_route(&facts_with_override("code", "deleted"), &registry).unwrap();
        assert_eq!(routing.profile, "implementer");
        assert_eq!(routing.matched_selector, "work_type=code");
    }

    #[test]
    fn override_with_no_matching_profile_and_no_routes_resolves_nothing() {
        let registry = registry("[profiles.a]\nclient = \"codex\"\nmodel = \"gpt-5.5\"\n");
        assert!(resolve_route(&facts_with_override("code", "ghost"), &registry).is_none());
    }

    #[test]
    fn empty_registry_parses_with_no_warnings() {
        let RegistryLoad::Loaded(registry) = parse_registry("") else {
            panic!("expected loaded registry");
        };
        assert!(registry.profiles.is_empty());
        assert!(registry.routes.is_empty());
        assert!(registry.route_default.is_none());
        assert!(validation_warnings(&registry).is_empty());
    }
}
