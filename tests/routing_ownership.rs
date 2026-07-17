use std::fs;
use std::path::{Path, PathBuf};

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
