use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const CONTRACT_PATH: &str = "docs/contracts/PI_INTEGRATION_V1.md";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn contract_text() -> String {
    fs::read_to_string(root().join(CONTRACT_PATH)).expect("Pi contract must be readable")
}

fn contract_json() -> Value {
    let text = contract_text();
    let body = text
        .split_once("```json\n")
        .and_then(|(_, tail)| tail.split_once("\n```"))
        .map(|(json, _)| json)
        .expect("Pi contract must contain one executable JSON fence");
    serde_json::from_str(body).expect("Pi contract JSON must parse")
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
fn pi_contract_maps_every_native_asset_exactly_once() {
    let contract = contract_json();
    let artifacts = contract["artifacts"]
        .as_array()
        .expect("artifacts must be an array");
    let mappings: BTreeSet<(String, String)> = artifacts
        .iter()
        .map(|artifact| {
            (
                artifact["source"].as_str().unwrap().to_string(),
                artifact["target"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(mappings.len(), artifacts.len(), "mappings must be unique");

    let expected_sources: BTreeSet<String> = relative_files(&root().join("plugins/planr/skills"))
        .into_iter()
        .map(|path| format!("plugins/planr/skills/{path}"))
        .chain(
            relative_files(&root().join("plugins/planr/agents/pi"))
                .into_iter()
                .map(|path| format!("plugins/planr/agents/pi/{path}")),
        )
        .collect();
    let mapped_sources: BTreeSet<String> =
        mappings.iter().map(|(source, _)| source.clone()).collect();
    assert_eq!(
        mapped_sources, expected_sources,
        "canonical Pi assets and contract drifted"
    );

    for (source, target) in mappings {
        let expected_target = if let Some(agent) = source.strip_prefix("plugins/planr/agents/pi/") {
            format!(".pi/agents/{agent}")
        } else {
            source.replacen("plugins/planr/", ".pi/", 1)
        };
        assert_eq!(target, expected_target, "Pi target mapping drifted");
        assert!(root().join(source).is_file(), "mapped source must exist");
    }
}

#[test]
fn pi_contract_is_native_trust_aware_and_optional_subagent_safe() {
    let contract = contract_json();
    assert_eq!(
        contract["mcp"],
        serde_json::json!({"supported": false, "artifacts": []})
    );
    assert_eq!(
        contract["extensions"],
        serde_json::json!({"emitted": false, "artifacts": []})
    );
    assert_eq!(
        contract["hooks"],
        serde_json::json!({"supported": false, "artifacts": []})
    );
    assert_eq!(
        contract["settings"],
        serde_json::json!({"emitted": false, "artifacts": []})
    );
    assert_eq!(contract["trust"]["project_resources_require_trust"], true);
    assert_eq!(
        contract["observed_client"],
        serde_json::json!({
            "environment": "PI_CODING_AGENT",
            "accepted_value": "true",
            "stored_value": "pi",
            "advisory_only": true
        })
    );
    assert_eq!(contract["client_all_includes_pi"], false);
    assert_eq!(
        contract["headless"]["command"],
        "pi --approve --model <provider/model> --thinking <level> -p \"<prompt>\""
    );

    for artifact in contract["artifacts"].as_array().unwrap() {
        let target = artifact["target"].as_str().unwrap();
        assert!(
            target.starts_with(".pi/skills/") || target.starts_with(".pi/agents/"),
            "Pi v1 owns only native skill and optional role paths: {target}"
        );
    }

    let text = contract_text();
    for required in [
        "Pi core deliberately ships without MCP, subagents, plan mode,",
        "Pi loads `.pi/skills/` only after the repository is trusted",
        "`pi-subagents` is optional",
        "mutation-capable `bash`",
        "`acceptanceRole: read-only`",
        "`completionGuard: false`",
        "Planr never writes",
        "Pi's user trust store or `.pi/settings.json`",
        "Run `planr prime` manually",
        "not authentication, authorization",
        "Release and CI workflows do not install/invoke Pi",
    ] {
        assert!(
            text.contains(required),
            "missing frozen Pi policy: {required}"
        );
    }
}

#[test]
fn pi_release_boundary_has_no_live_runtime_or_dependency() {
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
        for forbidden in ["pi --", "pi install", "pi-subagents", "PI_CODING_AGENT"] {
            assert!(
                !source.contains(forbidden),
                "{relative} must not invoke or configure the Pi runtime: {forbidden}"
            );
        }
    }

    let cargo = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let lock = fs::read_to_string(root().join("Cargo.lock")).unwrap();
    for package in ["pi", "pi-subagents"] {
        let marker = format!("name = \"{package}\"");
        assert!(
            !cargo.contains(&marker) && !lock.contains(&marker),
            "Pi support must not add a {package} Cargo dependency"
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
        "the npm package must retain the canonical assets compiled into Pi installs"
    );
    assert!(
        npm.get("dependencies").is_none_or(|dependencies| {
            dependencies.as_object().is_none_or(|dependencies| {
                !dependencies
                    .keys()
                    .any(|name| name == "pi" || name.contains("pi-subagents"))
            })
        }),
        "Pi support must not add an npm runtime dependency"
    );
}

#[test]
fn pi_product_and_public_docs_publish_the_frozen_contract() {
    let product = fs::read_to_string(root().join(".planr/plans/product/planr/PRODUCT_SPEC.md"))
        .expect("product spec must be readable");
    let architecture =
        fs::read_to_string(root().join(".planr/plans/product/planr/TECH_ARCHITECTURE.md"))
            .expect("technical architecture must be readable");
    let api = fs::read_to_string(root().join(".planr/plans/product/planr/API_AND_DATA_MODEL.md"))
        .expect("API contract must be readable");
    for (document, required) in [
        (&product, "legacy `all` selection excludes Grok and Pi"),
        (
            &architecture,
            "Pi: native `.pi/skills/` plus optional `pi-subagents` roles",
        ),
        (&api, "`PI_CODING_AGENT=true` child-process marker"),
    ] {
        assert!(
            document.contains(required),
            "canonical product contract is missing `{required}`"
        );
    }

    let guide = fs::read_to_string(root().join("apps/docs/content/docs/integrations/pi.mdx"))
        .expect("public Pi guide must be readable");
    for required in [
        "<AgentRecipe client=\"pi\" />",
        "not included by `--client all`",
        "writes no `.pi/settings.json`",
        "project is trusted",
        "pi --approve --model <provider/model>",
        "Use /skill:planr.",
        "PI_CODING_AGENT=true",
        "a597371bda2af70372d1323d550483b5f4a0ae36",
        "73fb2fc",
    ] {
        assert!(
            guide.contains(required),
            "public Pi guide is missing `{required}`"
        );
    }

    let typed_recipes = fs::read_to_string(root().join("apps/docs/lib/agent-recipes.ts"))
        .expect("typed agent recipes must be readable");
    for required in [
        "projectInstallerCommand: 'planr install pi'",
        "invocationLabel: '/skill:planr'",
        "Do not install a Pi package or pi-subagents",
    ] {
        assert!(
            typed_recipes.contains(required),
            "typed Pi recipe is missing `{required}`"
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
            .any(|page| page == "pi"),
        "the public Pi guide must be in integration navigation"
    );
}
