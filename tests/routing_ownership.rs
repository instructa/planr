use std::fs;
use std::path::{Path, PathBuf};

const REMOVED_ROUTING_PATHS: &[&str] = &[
    "planr-routing",
    "src/app/routing.rs",
    "src/routing_bundle.rs",
    "src/routing_bundle/tests.rs",
];

const FORBIDDEN_ROUTING_OWNER_NAMES: &[&str] = &[
    "planr-routing",
    "routing_bundle",
    "routing workflow skill",
    "src/app/routing.rs",
    "src/routing_bundle.rs",
    "src/routing_bundle/tests.rs",
];

const ROUTING_OWNER_SCAN_ROOTS: &[&str] = &[
    "src",
    "plugins/planr/skills",
    "docs",
    "README.md",
    "Cargo.toml",
    "package.json",
];

#[derive(Debug, PartialEq, Eq)]
struct ForbiddenHit {
    path: String,
    needle: &'static str,
}

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_none_or(|name| name != "tests.rs")
        {
            out.push(path);
        }
    }
}

fn scan_file(path: &Path, root: &Path, hits: &mut Vec<ForbiddenHit>) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for needle in FORBIDDEN_ROUTING_OWNER_NAMES {
        if text.contains(needle) {
            hits.push(ForbiddenHit {
                path: path.strip_prefix(root).unwrap().display().to_string(),
                needle,
            });
        }
    }
}

fn scan_path(path: &Path, root: &Path, hits: &mut Vec<ForbiddenHit>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            scan_path(&entry.unwrap().path(), root, hits);
        }
    } else if path.is_file() {
        scan_file(path, root, hits);
    }
}

fn forbidden_routing_owner_hits(root: &Path) -> Vec<ForbiddenHit> {
    let mut hits = Vec::new();
    for relative in ROUTING_OWNER_SCAN_ROOTS {
        let path = root.join(relative);
        if path.exists() {
            scan_path(&path, root, &mut hits);
        }
    }
    hits.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.needle.cmp(right.needle))
    });
    hits
}

#[test]
fn planr_core_contains_no_model_policy_or_legacy_preset_owner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for removed in [
        "src/preset.rs",
        "src/preset_catalog.rs",
        "src/preset_eval.rs",
        "src/preset_registry.rs",
        "src/app/presets.rs",
        "presets",
        "evaluations",
        "website",
    ] {
        assert!(
            !root.join(removed).exists(),
            "legacy owner returned: {removed}"
        );
    }

    let forbidden = [
        "gpt-5.6-",
        "gpt-5.5",
        "fable-5",
        "balanced",
        "low-usage",
        "max-quality",
        "read-only-audit",
        "codex-openai",
        "cursor-fable-grok",
        "preset_catalog",
        "preset_eval",
        "preset_registry",
        "agents preset",
    ];
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for needle in forbidden {
            assert!(
                !text.contains(needle),
                "Planr Core source {} contains package-owned policy `{needle}`",
                file.strip_prefix(root).unwrap().display()
            );
        }
    }
}

#[test]
fn planr_core_contains_no_legacy_routing_owner_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for removed in REMOVED_ROUTING_PATHS {
        assert!(
            !root.join(removed).exists(),
            "legacy routing owner path returned: {removed}"
        );
    }

    let hits = forbidden_routing_owner_hits(root);
    assert_eq!(
        hits,
        Vec::<ForbiddenHit>::new(),
        "legacy routing owner names must not appear in active Planr surfaces"
    );
}

#[test]
fn routing_owner_inventory_detector_catches_seeded_legacy_names() {
    let mut hits = Vec::new();
    let fixture_path = "seeded/legacy-routing-owner.txt".to_string();
    let seeded = FORBIDDEN_ROUTING_OWNER_NAMES.join("\n");
    for needle in FORBIDDEN_ROUTING_OWNER_NAMES {
        if seeded.contains(needle) {
            hits.push(ForbiddenHit {
                path: fixture_path.clone(),
                needle,
            });
        }
    }

    let expected = FORBIDDEN_ROUTING_OWNER_NAMES
        .iter()
        .map(|needle| ForbiddenHit {
            path: fixture_path.clone(),
            needle,
        })
        .collect::<Vec<_>>();
    assert_eq!(hits, expected);
}
