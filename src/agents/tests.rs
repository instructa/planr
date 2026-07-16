use super::*;

const VALID: &str = r#"
[profiles.cursor-implementer]
client = "cursor"
model = "fable-code"
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
profile = "cursor-implementer"
fallbacks = ["cursor-fable-driver"]

[route_default]
profile = "cursor-implementer"
"#;

#[test]
fn parses_profiles_routes_and_default() {
    let RegistryLoad::Loaded(registry) = parse_registry(VALID) else {
        panic!("expected loaded registry");
    };
    assert_eq!(registry.profiles.len(), 2);
    let implementer = &registry.profiles["cursor-implementer"];
    assert_eq!(implementer.client, "cursor");
    assert_eq!(implementer.model, "fable-code");
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
        Some("cursor-implementer")
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
    let text = "[profiles.a]\nclient = \"cursor\"\nmodel = \"fable-code\"\nefort = \"high\"\n";
    let RegistryLoad::Degraded { error } = parse_registry(text) else {
        panic!("expected degraded registry");
    };
    assert!(error.contains("efort"), "typo not named in: {error}");
}

#[test]
fn missing_required_profile_field_degrades() {
    let text = "[profiles.a]\nclient = \"cursor\"\n";
    let RegistryLoad::Degraded { error } = parse_registry(text) else {
        panic!("expected degraded registry");
    };
    assert!(error.contains("model"), "missing field not named: {error}");
}

#[test]
fn warns_on_unknown_profile_references() {
    let text = r#"
[profiles.a]
client = "cursor"
model = "fable-code"

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
client = "cursor"
model = "fable-code"

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
        "[profiles.a]\nclient = \"cursor\"\nmodel = \"fable-code\"\nnotes = \"key sk-abc123\"\n";
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
client = "cursor"
model = "fable-code"
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
    assert_eq!(routing.client, "cursor");
    assert_eq!(routing.model, "fable-code");
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
    let registry = registry("[profiles.a]\nclient = \"cursor\"\nmodel = \"fable-code\"\n");
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
client = "cursor"
model = "fable-code"

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
client = "cursor"
model = "fable-code"

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
    let registry = registry("[profiles.a]\nclient = \"cursor\"\nmodel = \"fable-code\"\n");
    assert!(resolve_route(&facts_with_override("code", "ghost"), &registry).is_none());
}

#[test]
fn codex_profiles_require_agent_type_and_never_resolve_through_fallback_chains() {
    let missing_agent_type = registry(
        "[profiles.worker]\nclient = \"codex\"\nmodel = \"gpt-5.6-terra\"\nagent_type = \"  \"\n",
    );
    let warnings = validation_warnings(&missing_agent_type);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("nonblank `agent_type`")),
        "{warnings:?}"
    );
    assert!(
        resolve_route(&facts_with_override("code", "worker"), &missing_agent_type).is_none(),
        "an override cannot make a non-current Codex profile usable"
    );

    let current = registry(
        r#"
[profiles.worker]
client = "codex"
model = "gpt-5.6-terra"
agent_type = "planr-terra-high"

[profiles.driver]
client = "cursor"
model = "fable-5"

[[routes]]
match = { work_type = "code" }
profile = "worker"
fallbacks = ["driver"]

[route_default]
profile = "driver"
fallbacks = ["worker"]
"#,
    );
    let warnings = validation_warnings(&current);
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning.contains("fallback chain involving Codex"))
            .count(),
        2,
        "{warnings:?}"
    );
    assert!(resolve_route(&facts("code", None), &current).is_none());
    assert!(resolve_route(&facts("docs", None), &current).is_none());

    let override_route = resolve_route(&facts_with_override("code", "worker"), &current)
        .expect("a direct current Codex override has no fallback chain");
    assert_eq!(override_route.agent_type, Some("planr-terra-high"));
    assert!(override_route.fallbacks.is_empty());
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
