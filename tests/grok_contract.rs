use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const CONTRACT_PATH: &str = "docs/contracts/GROK_INTEGRATION_V1.md";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn contract_text() -> String {
    fs::read_to_string(root().join(CONTRACT_PATH)).expect("Grok contract must be readable")
}

fn contract_json() -> Value {
    let text = contract_text();
    let body = text
        .split_once("```json\n")
        .and_then(|(_, tail)| tail.split_once("\n```"))
        .map(|(json, _)| json)
        .expect("Grok contract must contain one executable JSON fence");
    serde_json::from_str(body).expect("Grok contract JSON must parse")
}

fn relative_files(dir: &Path) -> BTreeSet<String> {
    fn visit(base: &Path, dir: &Path, files: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).expect("canonical asset directory must be readable") {
            let path = entry.expect("directory entry must be readable").path();
            if path.is_dir() {
                visit(base, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(base)
                        .expect("asset must be below base")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    let mut files = BTreeSet::new();
    visit(dir, dir, &mut files);
    files
}

#[test]
fn grok_contract_maps_every_canonical_workflow_asset_exactly_once() {
    let contract = contract_json();
    let artifacts = contract["artifacts"]
        .as_array()
        .expect("artifacts must be an array");

    let mappings: BTreeSet<(String, String)> = artifacts
        .iter()
        .filter_map(|artifact| {
            let source = artifact["source"].as_str()?;
            (!source.starts_with("generated:")).then(|| {
                (
                    source.to_string(),
                    artifact["target"].as_str().unwrap().to_string(),
                )
            })
        })
        .collect();
    assert_eq!(
        mappings.len(),
        artifacts.len() - 1,
        "asset mappings must be unique"
    );

    let expected_sources: BTreeSet<String> = relative_files(&root().join("plugins/planr/skills"))
        .into_iter()
        .map(|path| format!("plugins/planr/skills/{path}"))
        .chain(
            relative_files(&root().join("plugins/planr/agents"))
                .into_iter()
                .map(|path| format!("plugins/planr/agents/{path}")),
        )
        .collect();
    let mapped_sources: BTreeSet<String> =
        mappings.iter().map(|(source, _)| source.clone()).collect();
    assert_eq!(
        mapped_sources, expected_sources,
        "canonical assets and contract drifted"
    );

    for (source, target) in mappings {
        let expected_target = source.replacen("plugins/planr/", ".grok/", 1);
        assert_eq!(
            target, expected_target,
            "Grok target must mirror canonical source"
        );
        assert!(root().join(source).is_file(), "mapped source must exist");
    }
}

#[test]
fn grok_contract_is_portable_credential_free_and_plugin_free() {
    let contract = contract_json();
    let mcp = &contract["mcp"];

    assert_eq!(mcp["table"], "mcp_servers.planr");
    assert_eq!(mcp["command"], "planr");
    assert_eq!(mcp["args"], serde_json::json!(["mcp"]));
    assert_eq!(mcp["enabled"], true);
    assert_eq!(mcp["env"], serde_json::json!({"PLANR_MCP_CLIENT": "grok"}));
    assert_eq!(contract["plugin"]["emitted"], false);
    assert_eq!(contract["plugin"]["project_root"], ".grok/plugins");
    assert_eq!(
        contract["plugin"]["manifest_precedence"],
        serde_json::json!([
            "plugin.json",
            ".grok-plugin/plugin.json",
            ".claude-plugin/plugin.json",
            "convention"
        ])
    );
    assert_eq!(contract["plugin"]["project_default"], "disabled");
    assert_eq!(contract["plugin"]["project_config_enabled_merged"], false);
    assert_eq!(contract["hooks"]["supported"], false);
    assert_eq!(contract["client_all_includes_grok"], false);
    assert_eq!(
        contract["headless"]["command"],
        "grok --no-auto-update -p \"<prompt>\" --output-format json"
    );

    let machine_contract = serde_json::to_string(&contract).unwrap();
    for forbidden in [
        "xai-",
        "auth.json",
        "mcp_credentials.json",
        "--db",
        "grok exec",
    ] {
        assert!(
            !machine_contract.contains(forbidden),
            "machine contract must not contain forbidden value {forbidden}"
        );
    }
    for artifact in contract["artifacts"].as_array().unwrap() {
        assert!(
            !artifact["target"]
                .as_str()
                .unwrap()
                .starts_with(".grok/plugins/"),
            "v1 must use native skill/agent discovery, not a disabled project plugin"
        );
    }
}

#[test]
fn grok_contract_freezes_merge_observation_and_ci_policy() {
    let text = contract_text();
    for required in [
        "document-preserving TOML merge",
        "replace only that table",
        "Malformed config: fail",
        "PLANR_MCP_CLIENT=grok",
        "not authentication, authorization",
        "must not install or invoke Grok",
        "`grok inspect --json`",
        "`grok exec` is not part of this contract",
    ] {
        assert!(text.contains(required), "missing frozen policy: {required}");
    }
}

#[test]
fn grok_release_boundary_has_no_credentials_live_calls_or_runtime_dependency() {
    let forbidden = ["xai", "x.ai", "grok"];
    let production_release_files = [
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/workflows/security.yml",
        "scripts/build-release.sh",
        "scripts/build-linux-release.sh",
        "scripts/prepare-release-candidate.sh",
        "scripts/release.sh",
        "scripts/verify-linux-release-artifact.sh",
        "scripts/verify-public-lifecycle.sh",
    ];
    for relative in production_release_files {
        let source = fs::read_to_string(root().join(relative))
            .unwrap_or_else(|error| panic!("{relative} must be readable: {error}"));
        let normalized = source.to_ascii_lowercase();
        for value in forbidden {
            assert!(
                !normalized.contains(value),
                "{relative} must not contain credential or live Grok boundary `{value}`"
            );
        }
    }

    let cargo = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let lock = fs::read_to_string(root().join("Cargo.lock")).unwrap();
    for dependency in ["grok", "xai"] {
        let package_marker = format!("name = \"{dependency}\"");
        assert!(
            !cargo.contains(&package_marker) && !lock.contains(&package_marker),
            "Grok support must not add a {dependency} runtime dependency"
        );
    }

    let npm: Value =
        serde_json::from_str(&fs::read_to_string(root().join("package.json")).unwrap())
            .expect("package.json must parse");
    assert!(
        npm["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "plugins"),
        "the npm package must retain the canonical plugin assets compiled into Grok installs"
    );
    assert!(
        npm.get("dependencies").is_none_or(|dependencies| {
            dependencies.as_object().is_none_or(|dependencies| {
                !dependencies.keys().any(|name| {
                    let name = name.to_ascii_lowercase();
                    name.contains("grok") || name.contains("xai")
                })
            })
        }),
        "Grok support must not add an npm runtime dependency"
    );
}

#[test]
fn grok_product_and_public_docs_publish_the_frozen_user_contract() {
    let product = fs::read_to_string(root().join(".planr/plans/product/planr/PRODUCT_SPEC.md"))
        .expect("product spec must be readable");
    let architecture =
        fs::read_to_string(root().join(".planr/plans/product/planr/TECH_ARCHITECTURE.md"))
            .expect("technical architecture must be readable");
    let api = fs::read_to_string(root().join(".planr/plans/product/planr/API_AND_DATA_MODEL.md"))
        .expect("API contract must be readable");
    for (document, required) in [
        (&product, "legacy `all` selection excludes Grok"),
        (&architecture, "Grok Build has no Planr hook contract in v1"),
        (&api, "ambient `GROK_*` variables may not"),
    ] {
        assert!(
            document.contains(required),
            "canonical product contract is missing `{required}`"
        );
    }
    for relative in [
        ".planr/plans/product/planr/README.md",
        ".planr/plans/product/planr/AI_SPEC.md",
        "apps/docs/content/docs/index.mdx",
        "apps/docs/content/docs/getting-started/index.mdx",
        "apps/docs/content/docs/faq.mdx",
        "apps/docs/content/docs/getting-started/full-lifecycle.mdx",
    ] {
        let document = fs::read_to_string(root().join(relative)).unwrap();
        assert!(
            document.contains("Grok Build"),
            "authoritative client inventory omits Grok Build: {relative}"
        );
    }
    let docs_package: Value =
        serde_json::from_str(&fs::read_to_string(root().join("apps/docs/package.json")).unwrap())
            .expect("docs package must parse");
    assert!(
        docs_package["scripts"]["content"]
            .as_str()
            .unwrap()
            .contains("verify-grok-inventory.mjs"),
        "pnpm docs:content must enforce the Grok inventory"
    );
    let release_audit =
        fs::read_to_string(root().join("apps/docs/scripts/verify-release-readiness.mjs")).unwrap();
    assert!(release_audit.contains("six integration routes"));

    let guide =
        fs::read_to_string(root().join("apps/docs/content/docs/integrations/grok-build.mdx"))
            .expect("public Grok guide must be readable");
    for required in [
        "planr install grok --dry-run",
        "not included by `--client all`",
        "no Planr hooks in v1",
        "Disabling Claude-compatibility or plugin scanners",
        "grok inspect --json",
        "never install or invoke Grok",
        "never tokens, auth files, prompts, responses, transcripts, or private content",
        "b41c75a578f98bddbd326ab02cd53618451d97ee",
    ] {
        assert!(
            guide.contains(required),
            "public guide is missing `{required}`"
        );
    }

    let navigation: Value = serde_json::from_str(
        &fs::read_to_string(root().join("apps/docs/content/docs/integrations/meta.json")).unwrap(),
    )
    .expect("integration navigation must parse");
    assert!(
        navigation["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|page| page == "grok-build"),
        "the public Grok guide must be in integration navigation"
    );
}
