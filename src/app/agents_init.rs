//! `planr agents init`: writes the starter agent registry. Owns the
//! provider-neutral registry initialization plus the flag-driven spec
//! builder and interactive wizard — kept out of
//! `src/app/agents.rs` so the routing surface module stays within its
//! ownership budget.

use super::App;
use crate::agents::{REGISTRY_RELATIVE_PATH, registry_path};
use crate::cli::{AgentsInitArgs, InstallClientArgs, InstallCommand};
use crate::util::write_if_missing;
use anyhow::{Result, bail};
use serde_json::json;
use std::io::IsTerminal;

const PROFILE_GRAMMAR: &str = "--profile <id>=<client>/<model>[@<effort>][#<tier>]";
const SKILL_GRAMMAR: &str = "--skill <profile>=<skill>";
const ROUTE_GRAMMAR: &str = "--route <work_type>=<profile>[,<fallback>...]";
const DEFAULT_ROUTE_GRAMMAR: &str = "--default-route <profile>[,<fallback>...]";

#[derive(Debug, Clone)]
struct ProfileSpec {
    id: String,
    client: String,
    model: String,
    effort: Option<String>,
    cost_tier: Option<String>,
    skill: Option<String>,
}

#[derive(Debug, Clone)]
struct RouteSpec {
    work_type: String,
    profile: String,
    fallbacks: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct InitSpec {
    profiles: Vec<ProfileSpec>,
    routes: Vec<RouteSpec>,
    default_route: Option<(String, Vec<String>)>,
}

/// `<id>=<client>/<model>[@<effort>][#<tier>]`. `#` and `@` are stripped
/// right-to-left before the single client/model split, so a model id may
/// itself contain `/` (opencode's `provider/model-id` vocabulary).
/// Ids become bare TOML keys (`[profiles.<id>]`) and route selectors,
/// so they are restricted to bare-key-safe characters; values only need
/// to stay single-line (toml_str escapes quotes and backslashes, but a
/// TOML basic string cannot hold control characters).
fn check_spec_id(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "{label} `{value}` must contain only letters, digits, `-`, or `_` (it becomes a TOML key)"
        );
    }
    Ok(())
}

fn check_spec_value(label: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        bail!(
            "{label} `{}` must not contain control characters",
            value.escape_debug()
        );
    }
    Ok(())
}

fn parse_profile_spec(spec: &str) -> Result<ProfileSpec> {
    let Some((id, rest)) = spec.split_once('=') else {
        bail!("malformed profile spec `{spec}`; expected {PROFILE_GRAMMAR}");
    };
    let (rest, cost_tier) = match rest.rsplit_once('#') {
        Some((rest, tier)) => (rest, Some(tier)),
        None => (rest, None),
    };
    let (rest, effort) = match rest.rsplit_once('@') {
        Some((rest, effort)) => (rest, Some(effort)),
        None => (rest, None),
    };
    let Some((client, model)) = rest.split_once('/') else {
        bail!("malformed profile spec `{spec}`; expected {PROFILE_GRAMMAR}");
    };
    let parts = [
        ("id", id),
        ("client", client),
        ("model", model),
        ("effort", effort.unwrap_or("effort")),
        ("tier", cost_tier.unwrap_or("tier")),
    ];
    for (label, value) in parts {
        if value.trim().is_empty() {
            bail!("profile spec `{spec}` has an empty {label}; expected {PROFILE_GRAMMAR}");
        }
    }
    check_spec_id("profile id", id.trim())?;
    for (label, value) in [
        ("client", Some(client)),
        ("model", Some(model)),
        ("effort", effort),
        ("tier", cost_tier),
    ] {
        if let Some(value) = value {
            check_spec_value(label, value.trim())?;
        }
    }
    Ok(ProfileSpec {
        id: id.trim().to_string(),
        client: client.trim().to_string(),
        model: model.trim().to_string(),
        effort: effort.map(|value| value.trim().to_string()),
        cost_tier: cost_tier.map(|value| value.trim().to_string()),
        skill: None,
    })
}

fn parse_chain(value: &str) -> (String, Vec<String>) {
    let mut parts = value.split(',').map(|part| part.trim().to_string());
    let primary = parts.next().unwrap_or_default();
    (primary, parts.filter(|part| !part.is_empty()).collect())
}

fn parse_route_spec(spec: &str) -> Result<RouteSpec> {
    let Some((work_type, chain)) = spec.split_once('=') else {
        bail!("malformed route spec `{spec}`; expected {ROUTE_GRAMMAR}");
    };
    let (profile, fallbacks) = parse_chain(chain);
    if work_type.trim().is_empty() || profile.is_empty() {
        bail!("malformed route spec `{spec}`; expected {ROUTE_GRAMMAR}");
    }
    // Work types are free-form in the engine and render as TOML strings
    // (not keys), so only control characters are rejected here.
    check_spec_value("route work_type", work_type.trim())?;
    Ok(RouteSpec {
        work_type: work_type.trim().to_string(),
        profile,
        fallbacks,
    })
}

/// Compiles and cross-validates the flag specs, fail-closed: any unknown
/// profile reference or duplicate errors before a single byte is written,
/// so consistent inputs are guaranteed to produce a zero-warning registry.
fn compile_specs(args: &AgentsInitArgs) -> Result<InitSpec> {
    let mut spec = InitSpec::default();
    for profile in &args.profiles {
        let profile = parse_profile_spec(profile)?;
        if spec.profiles.iter().any(|known| known.id == profile.id) {
            bail!("duplicate profile id `{}`", profile.id);
        }
        spec.profiles.push(profile);
    }
    let known_ids: Vec<String> = spec
        .profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect();
    let known = |id: &str| known_ids.iter().any(|known| known == id);
    for pairing in &args.skills {
        let Some((profile_id, skill)) = pairing.split_once('=') else {
            bail!("malformed skill spec `{pairing}`; expected {SKILL_GRAMMAR}");
        };
        let (profile_id, skill) = (profile_id.trim(), skill.trim());
        if profile_id.is_empty() || skill.is_empty() {
            bail!("malformed skill spec `{pairing}`; expected {SKILL_GRAMMAR}");
        }
        check_spec_value("skill", skill)?;
        let Some(profile) = spec
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
        else {
            bail!("--skill references unknown profile `{profile_id}`; declare it with --profile");
        };
        profile.skill = Some(skill.to_string());
    }
    for route in &args.routes {
        let route = parse_route_spec(route)?;
        if spec
            .routes
            .iter()
            .any(|known| known.work_type == route.work_type)
        {
            bail!("duplicate route for work_type `{}`", route.work_type);
        }
        for referenced in std::iter::once(&route.profile).chain(route.fallbacks.iter()) {
            if !known(referenced) {
                bail!(
                    "--route {} references unknown profile `{referenced}`; declare it with --profile",
                    route.work_type
                );
            }
        }
        spec.routes.push(route);
    }
    if let Some(chain) = &args.default_route {
        let (profile, fallbacks) = parse_chain(chain);
        if profile.is_empty() {
            bail!("malformed default route `{chain}`; expected {DEFAULT_ROUTE_GRAMMAR}");
        }
        for referenced in std::iter::once(&profile).chain(fallbacks.iter()) {
            if !known(referenced) {
                bail!(
                    "--default-route references unknown profile `{referenced}`; declare it with --profile"
                );
            }
        }
        spec.default_route = Some((profile, fallbacks));
    }
    Ok(spec)
}

fn toml_str(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_chain(fallbacks: &[String]) -> String {
    fallbacks
        .iter()
        .map(|id| toml_str(id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a provider-neutral pool specification. Pure so flags and the wizard share it.
fn build_registry(spec: &InitSpec) -> String {
    let mut out = String::from(
        "# Planr agent profile registry: advisory model routing for pick packets.\n\
         # Planr never dispatches models; hosts stay the authority.\n\
         # Generated by `planr agents init`; edit freely — then re-run\n\
         # `planr agents check` and `planr install <client> --force`.\n\
         #\n\
         # Profiles and routes are opaque declarations. Host-specific policy\n\
         # and repository role artifacts arrive through routing bundles.\n",
    );
    for profile in &spec.profiles {
        out.push_str(&format!(
            "\n[profiles.{}]\nclient = {}\nmodel = {}\n",
            profile.id,
            toml_str(&profile.client),
            toml_str(&profile.model)
        ));
        if let Some(effort) = &profile.effort {
            out.push_str(&format!("effort = {}\n", toml_str(effort)));
        }
        if let Some(tier) = &profile.cost_tier {
            out.push_str(&format!("cost_tier = {}\n", toml_str(tier)));
        }
        if let Some(skill) = &profile.skill {
            out.push_str(&format!("skill = {}\n", toml_str(skill)));
        }
    }
    if !spec.routes.is_empty() {
        out.push_str(
            "\n# First matching route wins; per-item pins (`planr item route <id> --set`)\n# beat every route below.\n",
        );
    }
    for route in &spec.routes {
        out.push_str(&format!(
            "\n[[routes]]\nmatch = {{ work_type = {} }}\nprofile = {}\n",
            toml_str(&route.work_type),
            toml_str(&route.profile)
        ));
        if !route.fallbacks.is_empty() {
            out.push_str(&format!("fallbacks = [{}]\n", toml_chain(&route.fallbacks)));
        }
    }
    if let Some((profile, fallbacks)) = &spec.default_route {
        out.push_str(&format!(
            "\n[route_default]\nprofile = {}\n",
            toml_str(profile)
        ));
        if !fallbacks.is_empty() {
            out.push_str(&format!("fallbacks = [{}]\n", toml_chain(fallbacks)));
        }
    }
    out
}

const CLIENTS: [&str; 4] = ["codex", "claude-code", "cursor", "generic-mcp"];

/// The guided path: collects the same inputs the flags carry and feeds
/// the same builder. Returns None when the user cancels at the confirm
/// step (cliclack maps ESC/Ctrl-C to an interrupt error upstream).
fn wizard() -> Result<Option<InitSpec>> {
    cliclack::intro("planr agents init")?;
    cliclack::note(
        "Agent pool",
        "Declare who runs what: a premium driver keeps planning and verdicts,\nper-use-case profiles implement. Everything here is advisory —\nhosts stay the dispatch authority. (docs/MODEL_ROUTING.md)",
    )?;
    let mut spec = InitSpec::default();

    let driver_client: String = client_select("Driver session host (your strongest model)")?;
    let driver_model: String = cliclack::input("Driver model (alias or full id)")
        .placeholder("model-premium")
        .interact()?;
    let driver_effort: String = cliclack::input("Driver effort/reasoning tier")
        .default_input("high")
        .interact()?;
    spec.profiles.push(ProfileSpec {
        id: "driver".to_string(),
        client: driver_client,
        model: driver_model,
        effort: Some(driver_effort),
        cost_tier: Some("premium".to_string()),
        skill: None,
    });

    let use_cases: Vec<String> =
        cliclack::multiselect("Use cases to route (space to toggle; work types are free-form)")
            .item("backend".to_string(), "backend", "")
            .item("frontend".to_string(), "frontend", "")
            .item("design".to_string(), "design", "")
            .item("analysis".to_string(), "analysis", "token-hungry side work")
            .item("docs".to_string(), "docs", "")
            .initial_values(vec!["backend".to_string(), "frontend".to_string()])
            .interact()?;

    for use_case in &use_cases {
        cliclack::note(format!("Use case: {use_case}"), "Who implements it?")?;
        let client = client_select("Host")?;
        let model: String = cliclack::input("Model (alias or full id)")
            .placeholder("provider-model")
            .interact()?;
        let effort: String = cliclack::input("Effort/reasoning tier")
            .default_input("medium")
            .interact()?;
        let tier: String = cliclack::select("Cost tier")
            .item("standard".to_string(), "standard", "everyday implementer")
            .item("budget".to_string(), "budget", "token-hungry side work")
            .item("premium".to_string(), "premium", "")
            .interact()?;
        let skill: String = cliclack::input("Paired skill (empty for none)")
            .placeholder("planr-work")
            .required(false)
            .interact()?;
        let fall_back: bool = cliclack::confirm("Fall back to the driver on rate limits?")
            .initial_value(true)
            .interact()?;
        spec.profiles.push(ProfileSpec {
            id: use_case.clone(),
            client,
            model,
            effort: Some(effort),
            cost_tier: Some(tier),
            skill: (!skill.trim().is_empty()).then(|| skill.trim().to_string()),
        });
        spec.routes.push(RouteSpec {
            work_type: use_case.clone(),
            profile: use_case.clone(),
            fallbacks: if fall_back {
                vec!["driver".to_string()]
            } else {
                Vec::new()
            },
        });
    }

    // Verdicts stay premium: review routes to the driver unconditionally.
    spec.routes.push(RouteSpec {
        work_type: "review".to_string(),
        profile: "driver".to_string(),
        fallbacks: Vec::new(),
    });
    let mut default_select = cliclack::select("Default route (catches unmatched work)");
    for profile in &spec.profiles {
        default_select = default_select.item(
            profile.id.clone(),
            format!("{} ({}/{})", profile.id, profile.client, profile.model),
            "",
        );
    }
    let default_profile: String = default_select.interact()?;
    let default_fallbacks = if default_profile == "driver" {
        Vec::new()
    } else {
        vec!["driver".to_string()]
    };
    spec.default_route = Some((default_profile, default_fallbacks));

    cliclack::note(REGISTRY_RELATIVE_PATH, build_registry(&spec).trim_end())?;
    let confirmed = cliclack::confirm("Write this registry?")
        .initial_value(true)
        .interact()?;
    if !confirmed {
        cliclack::outro_cancel("nothing written")?;
        return Ok(None);
    }
    Ok(Some(spec))
}

fn client_select(prompt: &str) -> Result<String> {
    let mut select = cliclack::select(prompt);
    for client in CLIENTS {
        select = select.item(client.to_string(), client, "");
    }
    Ok(select.interact()?)
}

impl App {
    pub(crate) fn agents_init(&self, args: AgentsInitArgs) -> Result<()> {
        let has_spec_flags = !args.profiles.is_empty()
            || !args.skills.is_empty()
            || !args.routes.is_empty()
            || args.default_route.is_some();
        let path = registry_path(&self.root);
        if path.exists() && !args.force {
            bail!(
                "{REGISTRY_RELATIVE_PATH} already exists and is never overwritten; edit it directly or re-run with --force to replace it"
            );
        }
        let (mode, content, wizard_spec) = if args.interactive {
            if !std::io::stdin().is_terminal() {
                bail!(
                    "--interactive needs a terminal; use the flag grammar instead: {PROFILE_GRAMMAR} {SKILL_GRAMMAR} {ROUTE_GRAMMAR} {DEFAULT_ROUTE_GRAMMAR}"
                );
            }
            match wizard()? {
                Some(spec) => ("wizard", build_registry(&spec), Some(spec)),
                None => return Ok(()),
            }
        } else if has_spec_flags {
            if args.profiles.is_empty() {
                bail!("spec flags need at least one {PROFILE_GRAMMAR}");
            }
            ("flags", build_registry(&compile_specs(&args)?), None)
        } else {
            ("scaffold", build_registry(&InitSpec::default()), None)
        };
        // Defense in depth for generated content: never write a registry
        // that does not parse, whatever the builder inputs were.
        if let crate::agents::RegistryLoad::Degraded { error } =
            crate::agents::parse_registry(&content)
        {
            bail!(
                "generated registry does not parse ({error}); this is a planr bug — please report it"
            );
        }
        write_if_missing(&path, &content, args.force)?;
        self.record_event(
            "agents_registry_initialized",
            None,
            json!({"path": REGISTRY_RELATIVE_PATH, "forced": args.force, "mode": mode}),
        )?;
        if let Some(spec) = wizard_spec {
            self.wizard_outro(&spec)?;
            return Ok(());
        }
        self.emit(
            json!({
                "path": REGISTRY_RELATIVE_PATH,
                "created": true,
                "mode": mode,
                "next": ["edit .planr/agents.toml or apply a routing bundle", "planr agents check"],
            }),
            format!("wrote provider-neutral {REGISTRY_RELATIVE_PATH}\nnext: edit it or apply a routing bundle, then run `planr agents check`"),
        )
    }

    /// Post-write wizard steps: optional role-file installs for the
    /// clients the pool actually uses, then the follow-up commands.
    fn wizard_outro(&self, spec: &InitSpec) -> Result<()> {
        let mut clients: Vec<String> = spec
            .profiles
            .iter()
            .map(|profile| profile.client.clone())
            .filter(|client| ["codex", "claude-code", "cursor"].contains(&client.as_str()))
            .collect();
        clients.sort();
        clients.dedup();
        let mut installs = Vec::new();
        if !clients.is_empty() {
            let mut select = cliclack::multiselect(
                "Render host role files now? (space to toggle, enter to confirm)",
            )
            .required(false);
            for client in &clients {
                select = select.item(client.clone(), client, "");
            }
            installs = select.interact()?;
        }
        cliclack::outro(format!(
            "wrote {REGISTRY_RELATIVE_PATH}; next: `planr agents check`, then `planr install <client> --force` after edits"
        ))?;
        for client in installs {
            let install_args = InstallClientArgs {
                dry_run: false,
                no_mcp: true,
                force: true,
                no_hooks: false,
            };
            self.install(match client.as_str() {
                "codex" => InstallCommand::Codex(install_args),
                "claude-code" => InstallCommand::Claude(install_args),
                _ => InstallCommand::Cursor(install_args),
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{RegistryLoad, parse_registry, validation_warnings};

    fn args(
        profiles: &[&str],
        skills: &[&str],
        routes: &[&str],
        default: Option<&str>,
    ) -> AgentsInitArgs {
        AgentsInitArgs {
            force: false,
            profiles: profiles.iter().map(ToString::to_string).collect(),
            skills: skills.iter().map(ToString::to_string).collect(),
            routes: routes.iter().map(ToString::to_string).collect(),
            default_route: default.map(ToString::to_string),
            interactive: false,
        }
    }

    #[test]
    fn full_spec_compiles_and_builds_a_zero_warning_registry() {
        let spec = compile_specs(&args(
            &[
                "driver=cursor/model-premium@high#premium",
                "designer=claude-code/model-review@high#premium",
                "backender=cursor/model-standard@high#standard",
            ],
            &["designer=frontend-design", "backender=planr-work"],
            &[
                "frontend=designer,driver",
                "backend=backender,driver",
                "review=driver",
            ],
            Some("backender,driver"),
        ))
        .unwrap();
        let content = build_registry(&spec);
        assert!(!content.contains("codex exec"));
        let RegistryLoad::Loaded(registry) = parse_registry(&content) else {
            panic!("generated registry failed to parse:\n{content}");
        };
        assert!(validation_warnings(&registry).is_empty());
        assert_eq!(registry.profiles.len(), 3);
        assert_eq!(
            registry.profiles["designer"].skill.as_deref(),
            Some("frontend-design")
        );
        assert_eq!(registry.routes.len(), 3);
        assert_eq!(
            registry.route_default.as_ref().unwrap().fallbacks,
            ["driver"]
        );
    }

    #[test]
    fn unsafe_ids_and_values_fail_closed_before_any_write() {
        // Dotted/space ids would become nested or invalid TOML table keys;
        // control characters would break TOML basic strings.
        for (args, needle) in [
            (
                args(&["team.alpha=cursor/m"], &[], &[], None),
                "must contain only letters",
            ),
            (
                args(&["bad id=cursor/m"], &[], &[], None),
                "must contain only letters",
            ),
            (
                args(&["a=cursor/m\nnewline"], &[], &[], None),
                "control characters",
            ),
            (
                args(&["a=cursor/m"], &[], &["qa\nmanual=a"], None),
                "control characters",
            ),
            (
                args(&["a=cursor/m"], &["a=skill\nname"], &[], None),
                "control characters",
            ),
        ] {
            let error = compile_specs(&args).unwrap_err().to_string();
            assert!(error.contains(needle), "expected `{needle}` in `{error}`");
        }
    }

    #[test]
    fn opencode_style_model_ids_keep_their_slash() {
        let profile =
            parse_profile_spec("runner=generic-mcp/anthropic/claude-sonnet@low#budget").unwrap();
        assert_eq!(profile.client, "generic-mcp");
        assert_eq!(profile.model, "anthropic/claude-sonnet");
        assert_eq!(profile.effort.as_deref(), Some("low"));
        assert_eq!(profile.cost_tier.as_deref(), Some("budget"));
    }

    #[test]
    fn host_names_remain_opaque_to_core() {
        let spec = compile_specs(&args(
            &["worker=host-a/model-primary@high#standard"],
            &[],
            &[],
            None,
        ))
        .unwrap();
        assert_eq!(spec.profiles[0].client, "host-a");
    }

    #[test]
    fn specs_fail_closed_with_grammar_in_the_message() {
        for (args, needle) in [
            (args(&["noequals"], &[], &[], None), "--profile <id>="),
            (args(&["a=cursor"], &[], &[], None), "--profile <id>="),
            (
                args(&["a=cursor/m", "a=claude-code/m"], &[], &[], None),
                "duplicate profile id",
            ),
            (
                args(&["a=cursor/m"], &["nosuch=skill"], &[], None),
                "unknown profile `nosuch`",
            ),
            (
                args(&["a=cursor/m"], &[], &["code=nosuch"], None),
                "unknown profile `nosuch`",
            ),
            (
                args(&["a=cursor/m"], &[], &["code=a", "code=a"], None),
                "duplicate route",
            ),
            (
                args(&["a=cursor/m"], &[], &[], Some("nosuch")),
                "unknown profile `nosuch`",
            ),
        ] {
            let error = compile_specs(&args).unwrap_err().to_string();
            assert!(error.contains(needle), "expected `{needle}` in `{error}`");
        }
    }
}
