use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const FIXTURE_ROOT: &str = "tests/fixtures/evidence/host-capabilities/v1";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn native_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        other => panic!("unsupported test platform: {other:?}"),
    }
}

fn replay(root: &Path) -> std::process::Output {
    replay_with_env(root, &[])
}

fn replay_command(root: &Path) -> Command {
    let mut command = Command::new("node");
    command.current_dir(repo_root()).args([
        "scripts/host-capability-experiment.mjs",
        "replay",
        "--fixture-root",
        root.to_str().unwrap(),
    ]);
    command
}

fn replay_with_env(root: &Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut command = replay_command(root);
    command.env(
        "PLANR_HOST_CAPABILITY_VALIDATOR",
        env!("CARGO_BIN_EXE_planr-host-capability-validator"),
    );
    for (key, value) in env {
        command.env(key, value);
    }
    command
        .output()
        .expect("host capability replay script must run")
}

fn replay_without_validator_env(root: &Path) -> std::process::Output {
    let mut command = replay_command(root);
    command
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .output()
        .expect("host capability replay script must run")
}

fn replay_with_packaged_layout(root: &Path, package_dir: &Path) -> std::process::Output {
    let scripts_dir = package_dir.join("scripts");
    let fixture_dir = package_dir.join("tests/fixtures/evidence/host-capabilities/v1");
    fs::create_dir_all(&scripts_dir).unwrap();
    copy_dir(root, &fixture_dir);
    let harness = scripts_dir.join("host-capability-experiment.mjs");
    fs::copy(
        repo_root().join("scripts/host-capability-experiment.mjs"),
        &harness,
    )
    .unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_planr-host-capability-validator"),
        scripts_dir.join("planr-host-capability-validator"),
    )
    .unwrap();
    make_executable(&scripts_dir.join("planr-host-capability-validator"));
    let mut command = Command::new("node");
    command
        .current_dir(package_dir)
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .args([
            harness.to_str().unwrap(),
            "replay",
            "--fixture-root",
            fixture_dir.to_str().unwrap(),
        ])
        .output()
        .expect("packaged host capability replay script must run")
}

fn capture(root: &Path) -> std::process::Output {
    Command::new("node")
        .current_dir(repo_root())
        .env(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            env!("CARGO_BIN_EXE_planr-host-capability-validator"),
        )
        .args([
            "scripts/host-capability-experiment.mjs",
            "capture",
            "--out-dir",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("host capability capture script must run")
}

fn capture_import(root: &Path, import_root: &Path) -> std::process::Output {
    Command::new("node")
        .current_dir(repo_root())
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .args([
            "scripts/host-capability-experiment.mjs",
            "capture",
            "--out-dir",
            root.to_str().unwrap(),
            "--import-fixture-root",
            import_root.to_str().unwrap(),
        ])
        .output()
        .expect("host capability capture import script must run")
}

fn capture_with_peer_probe_commands(
    root: &Path,
    import_root: Option<&Path>,
    overrides: &[(&str, &Path)],
) -> std::process::Output {
    let driver_root = tempdir().unwrap();
    let driver = driver_root.path().join("capture-driver.mjs");
    fs::write(
        &driver,
        r#"import { pathToFileURL } from "node:url";
const harness = await import(pathToFileURL(process.env.PLANR_TEST_HARNESS).href);
const suite = harness.defaultSuite();
const overrides = JSON.parse(process.env.PLANR_TEST_PEER_PROBES);
for (const [experimentId, command] of Object.entries(overrides)) {
  const experiment = suite.experiments.find((candidate) => candidate.id === experimentId);
  if (!experiment) throw new Error(`missing experiment ${experimentId}`);
  experiment.expected_tool_name = command;
}
const importFixtureRoot = process.env.PLANR_TEST_IMPORT_ROOT || undefined;
await harness.capture(
  process.env.PLANR_TEST_CAPTURE_OUT,
  importFixtureRoot ? { importFixtureRoot } : {},
  suite,
);
"#,
    )
    .unwrap();
    let override_json = Value::Object(
        overrides
            .iter()
            .map(|(id, command)| {
                (
                    (*id).to_string(),
                    json!(format!("{} --version", command.display())),
                )
            })
            .collect(),
    );
    let mut command = Command::new("node");
    command
        .current_dir(repo_root())
        .env(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            env!("CARGO_BIN_EXE_planr-host-capability-validator"),
        )
        .env(
            "PLANR_TEST_HARNESS",
            repo_root().join("scripts/host-capability-experiment.mjs"),
        )
        .env("PLANR_TEST_CAPTURE_OUT", root)
        .env("PLANR_TEST_PEER_PROBES", override_json.to_string())
        .arg(&driver);
    if let Some(import_root) = import_root {
        command.env("PLANR_TEST_IMPORT_ROOT", import_root);
    }
    command
        .output()
        .expect("host capability probe downgrade driver must run")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).unwrap();
        }
    }
}

fn read_json(root: &Path, relative: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(relative)).unwrap()).unwrap()
}

fn read_host_capability_readme(package_root: &Path) -> String {
    fs::read_to_string(package_root.join("tests/fixtures/evidence/host-capabilities/README.md"))
        .unwrap()
}

fn host_capability_doc_blocks(package_root: &Path) -> Vec<String> {
    let readme = read_host_capability_readme(package_root);
    let mut blocks = Vec::new();
    let mut rest = readme.as_str();
    while let Some(start) = rest.find("```bash\n") {
        let after_start = &rest[start + "```bash\n".len()..];
        let Some(end) = after_start.find("\n```") else {
            panic!("unterminated host capability README bash block");
        };
        blocks.push(after_start[..end].to_string());
        rest = &after_start[end + "\n```".len()..];
    }
    blocks
}

fn assert_host_capability_doc_snippets(package_root: &Path) -> Vec<String> {
    let readme = read_host_capability_readme(package_root);
    let blocks = host_capability_doc_blocks(package_root);
    assert_eq!(
        blocks.len(),
        3,
        "host capability README must publish replay, capture, and import bash blocks"
    );
    assert!(
        !readme.contains("/tmp/planr-host-capabilities")
            && !readme.contains("/tmp/host-external-envelope"),
        "host capability README must not document symlink-prone /tmp capture paths"
    );
    assert!(
        !readme.contains("node scripts/host-capability-experiment.mjs")
            && !readme.contains("--fixture-root tests/fixtures/evidence/host-capabilities/v1")
            && !readme.contains("cd \"$1\""),
        "host capability README must not rely on caller cwd or hidden test cd"
    );
    assert!(
        readme.contains("PLANR_PACKAGE_ROOT")
            && readme.matches(": \"${PLANR_PACKAGE_ROOT:?").count() == 3
            && readme.matches("pwd -P").count() >= 4
            && readme.contains("planr_package_root")
            && readme.contains("planr_harness")
            && readme.contains("planr_fixture_root")
            && readme.contains("\"$planr_harness\"")
            && readme.contains("\"$planr_fixture_root\""),
        "host capability README must document and canonicalize PLANR_PACKAGE_ROOT"
    );
    assert!(
        blocks
            .iter()
            .all(|block| block.contains(": \"${PLANR_PACKAGE_ROOT:?")
                && block.contains(
                    "planr_package_root=\"$(CDPATH=; cd -- \"$PLANR_PACKAGE_ROOT\" && pwd -P)\""
                )
                && block.contains(
                    "planr_harness=\"$planr_package_root/scripts/host-capability-experiment.mjs\""
                )),
        "each published block must validate and canonicalize PLANR_PACKAGE_ROOT"
    );
    assert!(
        readme.contains("planr_capture_tmp")
            && readme.contains("planr_capture_out")
            && readme.contains("planr_import_root"),
        "host capability README must use task-specific shell variables"
    );
    assert!(
        !readme.contains("rm -rf /tmp")
            && !readme.contains("rm -rf \"$tmp_root\"")
            && readme.matches("rm -rf \"$planr_capture_tmp\"").count() == 2,
        "host capability README cleanup must target only the exact mktemp directory"
    );
    assert!(
        readme.contains("fixtureLocalFunction(40, 2)")
            && readme.contains("imported local-function artifact bytes drifted")
            && readme.contains("external_producer")
            && !readme.contains("observed/exp-chrome-browser-client.json"),
        "import example must prove fresh local-function bytes/digest/provenance without relabeling archived Chrome"
    );
    blocks
}

fn run_host_capability_doc_snippet(package_root: &Path, snippet: &str) {
    let arbitrary_cwd = tempdir().unwrap();
    let tmp_parent = tempdir().unwrap();
    let physical_tmp_parent = fs::canonicalize(tmp_parent.path()).unwrap();
    let output = Command::new("sh")
        .current_dir(arbitrary_cwd.path())
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .env("PLANR_PACKAGE_ROOT", package_root)
        .env("TMPDIR", &physical_tmp_parent)
        .args(["-eu", "-c", snippet, "host-capability-doc-smoke"])
        .output()
        .expect("host capability documented snippet must run");
    assert!(
        output.status.success(),
        "documented host capability snippet failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let leftovers: Vec<_> = fs::read_dir(&physical_tmp_parent).unwrap().collect();
    assert!(
        leftovers.is_empty(),
        "documented cleanup must remove only the exact mktemp directory; leftovers: {leftovers:?}"
    );
}

fn run_host_capability_doc_smoke(package_root: &Path) {
    for block in assert_host_capability_doc_snippets(package_root) {
        run_host_capability_doc_snippet(package_root, &block);
    }
}

fn write_json(root: &Path, relative: &str, value: &Value) {
    fs::write(
        root.join(relative),
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    make_executable(path);
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn wait_process_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        sleep(Duration::from_millis(50));
    }
    !process_exists(pid)
}

fn read_pid(path: &Path) -> u32 {
    fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap()
}

fn sha256_file(path: &Path) -> String {
    format!("sha256:{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
        .collect()
}

fn current_utc_second() -> String {
    format_utc_second(OffsetDateTime::now_utc())
}

fn format_utc_second(value: OffsetDateTime) -> String {
    value
        .replace_nanosecond(0)
        .unwrap()
        .format(&Rfc3339)
        .unwrap()
        .replace("+00:00", "Z")
}

fn minimal_png_bytes() -> Vec<u8> {
    hex_bytes(
        "89504e470d0a1a0a0000000d4948445200000001000000010802000000907753de\
         0000000c49444154789c63f8cfc0000003010100c9fe92ef0000000049454e44ae426082",
    )
}

fn refresh_expected_schema_digest(root: &Path, schema_key: &str, schema_path: &str) {
    let mut expected = read_json(root, "expected/normalized-manifest.json");
    expected["schema_refs"][schema_key]["digest"] = json!(sha256_file(&root.join(schema_path)));
    write_json(root, "expected/normalized-manifest.json", &expected);
}

fn refresh_provenance_digest(root: &Path) {
    let mut expected = read_json(root, "expected/normalized-manifest.json");
    let digest = sha256_file(&root.join("provenance/host-capability-captures.json"));
    expected["provenance_ref"]["digest"] = json!(digest);
    let provenance_ref = expected["provenance_ref"].clone();
    for entry in expected["capability_instances"].as_array_mut().unwrap() {
        entry["provenance_ref"] = provenance_ref.clone();
    }
    for entry in fs::read_dir(root.join("observed")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            let relative = format!("observed/{}", path.file_name().unwrap().to_str().unwrap());
            let mut raw = read_json(root, &relative);
            raw["provenance_ref"] = provenance_ref.clone();
            write_json(root, &relative, &raw);
        }
    }
    write_json(root, "expected/normalized-manifest.json", &expected);
    refresh_host_surface_matrix(root);
}

fn refresh_host_surface_matrix(root: &Path) {
    let expected = read_json(root, "expected/normalized-manifest.json");
    let provenance = read_json(root, "provenance/host-capability-captures.json");
    let provenance_by_id = provenance["captures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|capture| {
            (
                capture["experiment_id"].as_str().unwrap().to_string(),
                capture.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let surfaces = expected["capability_instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let raw_capture_id = entry["raw_capture_id"].as_str().unwrap();
            let raw = read_json(root, &format!("observed/{raw_capture_id}.json"));
            let instance = &entry["capability_instance"];
            let provenance = provenance_by_id.get(raw_capture_id).unwrap();
            let mut artifact_kinds = raw["result"]["artifact_refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|artifact| artifact["kind"].clone())
                .collect::<Vec<_>>();
            artifact_kinds.sort_by_key(|kind| kind.as_str().unwrap().to_string());
            json!({
                "host": instance["host"],
                "surface": instance["surface"],
                "host_version": instance["host_version"],
                "trusted_adapter_enabled": entry["trusted_adapter_enabled"],
                "availability_status": instance["availability"]["status"],
                "reason": instance["availability"]["reason"],
                "observation_types": instance["observed_payload_contract"]["observation_types"],
                "provenance": {
                    "claim_source": entry["claim_source"],
                    "source_kind": provenance["source_kind"],
                    "observation_mode": provenance["observation_mode"],
                },
                "permissions": instance["permissions"],
                "artifact_kinds": artifact_kinds,
                "blind_spots": instance["limitations"],
            })
        })
        .collect::<Vec<_>>();
    write_json(
        root,
        "expected/host-surface-matrix.json",
        &json!({
            "schema_version": "planr.host_surface_capability_matrix.v1",
            "fixture_contract": "host-capability-raw/1.0.0",
            "suite_id": expected["suite_id"],
            "surfaces": surfaces,
        }),
    );
}

fn set_chrome_availability_reason(root: &Path, reason: &str) {
    let mut raw = read_json(root, "observed/exp-chrome-browser-client.json");
    raw["result"]["availability_reason"] = json!(reason);
    write_json(root, "observed/exp-chrome-browser-client.json", &raw);

    let mut provenance = read_json(root, "provenance/host-capability-captures.json");
    let capture = provenance["captures"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|capture| capture["experiment_id"] == "exp-chrome-browser-client")
        .unwrap();
    capture["availability_reason"] = json!(reason);
    write_json(
        root,
        "provenance/host-capability-captures.json",
        &provenance,
    );
    refresh_provenance_digest(root);

    let mut expected = read_json(root, "expected/normalized-manifest.json");
    let entry = expected_entry_mut(&mut expected, "exp-chrome-browser-client");
    entry["capability_instance"]["availability"]["reason"] = json!(reason);
    write_json(root, "expected/normalized-manifest.json", &expected);
    refresh_host_surface_matrix(root);
}

fn add_chrome_screenshot_artifact(root: &Path, kind: &str, relative_path: &str, bytes: &[u8]) {
    let artifact_path = root.join(relative_path);
    fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    fs::write(&artifact_path, bytes).unwrap();
    let digest = sha256_file(&artifact_path);
    let artifact_id = "artifact-chrome-browser-client-screenshot";

    let mut raw = read_json(root, "observed/exp-chrome-browser-client.json");
    raw["result"]["artifact_refs"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": artifact_id,
            "kind": kind,
            "root_kind": "fixture_root",
            "path": relative_path,
            "digest": digest,
        }));
    raw["result"]["artifact_digests"][artifact_id] = json!(digest);
    write_json(root, "observed/exp-chrome-browser-client.json", &raw);

    let mut provenance = read_json(root, "provenance/host-capability-captures.json");
    let capture = provenance["captures"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|capture| capture["experiment_id"] == "exp-chrome-browser-client")
        .unwrap();
    capture["artifact_ids"]
        .as_array_mut()
        .unwrap()
        .push(json!(artifact_id));
    write_json(
        root,
        "provenance/host-capability-captures.json",
        &provenance,
    );
    refresh_provenance_digest(root);
}

fn mutated_fixture(mutator: impl FnOnce(&Path)) -> PathBuf {
    let dir = tempdir().unwrap();
    let path = dir.keep();
    copy_dir(&repo_root().join(FIXTURE_ROOT), &path);
    mutator(&path);
    path
}

fn observed_bundle(mutator: impl FnOnce(&Path)) -> PathBuf {
    let dir = tempdir().unwrap();
    let path = dir.keep();
    let fixture_root = repo_root().join(FIXTURE_ROOT);
    let captured_at = current_utc_second();
    copy_dir(
        &fixture_root.join("artifacts/chrome-browser-client"),
        &path.join("artifacts/chrome-browser-client"),
    );
    let captures: Vec<Value> = ["exp-chrome-browser-client"]
        .iter()
        .map(|id| {
            let mut raw = read_json(&fixture_root, &format!("observed/{id}.json"));
            raw["started_at"] = json!(captured_at);
            raw["ended_at"] = json!(captured_at);
            raw
        })
        .collect();
    write_json(
        &path,
        "external-capture-envelope.json",
        &json!({
            "schema_version": "planr.host_capability_external_capture_envelope.v1",
            "producer": {
                "name": "committed-host-observation-fixture",
                "version": "1.0.0",
                "captured_at": captured_at
            },
            "suite_id": "codex-host-capability-phase1",
            "captures": captures
        }),
    );
    mutator(&path);
    path
}

fn mutate_external_capture(root: &Path, mutator: impl FnOnce(&mut Value)) {
    let mut envelope = read_json(root, "external-capture-envelope.json");
    let raw = envelope["captures"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|capture| capture["experiment_id"] == "exp-chrome-browser-client")
        .unwrap();
    mutator(raw);
    write_json(root, "external-capture-envelope.json", &envelope);
}

fn valid_screenshot_claim_fixture() -> PathBuf {
    mutated_fixture(|root| {
        let bytes = minimal_png_bytes();
        add_chrome_screenshot_artifact(
            root,
            "screenshot",
            "artifacts/chrome-browser-client/validator-bound-screenshot.png",
            &bytes,
        );
        set_chrome_availability_reason(
            root,
            "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot digest observations",
        );
    })
}

fn assert_replay_rejects(root: &Path, expected: &str) {
    let output = replay(root);
    assert!(
        !output.status.success(),
        "replay unexpectedly passed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "stderr did not contain {expected:?}: {stderr}"
    );
}

fn expected_entry_mut<'a>(expected: &'a mut Value, raw_capture_id: &str) -> &'a mut Value {
    expected["capability_instances"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|entry| entry["raw_capture_id"] == raw_capture_id)
        .unwrap()
}

#[test]
fn host_capability_experiment_replays_observed_fixtures() {
    let output = replay(&repo_root().join(FIXTURE_ROOT));
    assert!(
        output.status.success(),
        "replay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        summary["schema_version"],
        "planr.host_capability_experiment_summary.v1"
    );
    assert_eq!(summary["verdict"], "pass");
    assert_eq!(summary["experiment_count"], 14);
    assert_eq!(
        summary["availability"]["exp-terminal"],
        json!({
            "availability_status": "available",
            "trusted_adapter_enabled": false
        })
    );
    assert_eq!(
        summary["availability"]["exp-mcp-browser"]["availability_status"],
        "degraded"
    );
    assert_eq!(
        summary["availability"]["exp-computer-use"]["availability_status"],
        "permission_denied"
    );
    assert_eq!(
        summary["availability"]["exp-chrome-browser-client"]["availability_status"],
        "available"
    );
    for peer_id in [
        "exp-claude-code-host-capture",
        "exp-cursor-agent-host-capture",
        "exp-pi-cli-host-capture",
    ] {
        assert_eq!(
            summary["availability"][peer_id],
            json!({
                "availability_status": "unavailable",
                "trusted_adapter_enabled": false
            }),
            "{peer_id} must remain unavailable until a capture payload contract is observed"
        );
    }
    let expected = read_json(
        &repo_root().join(FIXTURE_ROOT),
        "expected/normalized-manifest.json",
    );
    let peer_versions = [
        (
            "exp-claude-code-host-capture",
            "claude",
            "claude-code",
            "2.1.133",
        ),
        (
            "exp-cursor-agent-host-capture",
            "cursor",
            "cursor-agent",
            "2026.07.17-3e2a980",
        ),
        ("exp-pi-cli-host-capture", "pi", "pi-cli", "0.82.1"),
    ];
    for (id, host, surface, version) in peer_versions {
        let entry = expected["capability_instances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["raw_capture_id"] == id)
            .unwrap();
        assert_eq!(entry["claim_source"], "mechanical_unavailable_probe");
        assert_eq!(entry["trusted_adapter_enabled"], false);
        assert_eq!(entry["capability_instance"]["host"], host);
        assert_eq!(entry["capability_instance"]["surface"], surface);
        assert_eq!(entry["capability_instance"]["host_version"], version);
        assert_eq!(
            entry["capability_instance"]["availability"]["status"],
            "unavailable"
        );
        assert!(
            entry["capability_instance"]["availability"]["reason"]
                .as_str()
                .unwrap()
                .contains("mechanical version probe returned"),
            "{entry}"
        );
        let raw = read_json(
            &repo_root().join(FIXTURE_ROOT),
            &format!("observed/{id}.json"),
        );
        let artifact_kinds = raw["result"]["artifact_refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|artifact| artifact["kind"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        for kind in [
            "invocation-stdout",
            "invocation-stderr",
            "invocation-result",
        ] {
            assert!(
                artifact_kinds.contains(kind),
                "{id} missing mechanical {kind} artifact"
            );
        }
    }
    let matrix = read_json(
        &repo_root().join(FIXTURE_ROOT),
        "expected/host-surface-matrix.json",
    );
    assert_eq!(
        matrix["schema_version"],
        "planr.host_surface_capability_matrix.v1"
    );
    let matrix_surfaces = matrix["surfaces"].as_array().unwrap();
    assert_eq!(matrix_surfaces.len(), 14);
    assert!(
        matrix_surfaces
            .iter()
            .all(|entry| entry.get("status").is_none())
    );
    let degraded = matrix_surfaces
        .iter()
        .find(|entry| entry["surface"] == "mcp-browser")
        .unwrap();
    assert_eq!(degraded["availability_status"], "degraded");
    let peer = matrix_surfaces
        .iter()
        .find(|entry| entry["host"] == "claude" && entry["surface"] == "claude-code")
        .unwrap();
    assert_eq!(peer["trusted_adapter_enabled"], false);
    assert_eq!(
        peer["provenance"]["observation_mode"],
        "mechanical_invocation"
    );
}

#[test]
fn host_surface_matrix_rejects_handwritten_degradation_permission_payload_and_upgrade_drift() {
    for (field, value) in [
        ("availability_status", json!("available")),
        ("permissions", json!({"network": "silently-expanded"})),
        ("observation_types", json!(["host.forged.payload.v2"])),
        ("host_version", json!("999.0.0")),
    ] {
        let drifted = mutated_fixture(|root| {
            let mut matrix = read_json(root, "expected/host-surface-matrix.json");
            let surface = matrix["surfaces"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["surface"] == "mcp-browser")
                .unwrap();
            surface[field] = value;
            write_json(root, "expected/host-surface-matrix.json", &matrix);
        });
        assert_replay_rejects(
            &drifted,
            "expected host-surface matrix drifted from verified manifests and fixtures",
        );
    }
}

#[test]
fn host_capability_experiment_rejects_drift_and_forgery() {
    let unknown_major = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["schema_version"] = json!("planr.host_capability_observed_raw.v2");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(&unknown_major, "unsupported major version 2");

    let missing_final = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["events"][1].as_object_mut().unwrap().remove("final");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(&missing_final, "exactly one final event");

    let forged_tool = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["tool_name"] = json!("functions.fake_exec");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(&forged_tool, "tool_name is forged or drifted");

    let forged_artifact = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["result"]["artifact_refs"][0]["digest"] =
            json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(
        &forged_artifact,
        "artifact artifact-terminal-stdout digest mismatch",
    );

    let peer_version_drift = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-claude-code-host-capture.json");
        raw["host_identity"]["version"] = json!("2.1.134");
        write_json(root, "observed/exp-claude-code-host-capture.json", &raw);
    });
    assert_replay_rejects(
        &peer_version_drift,
        "host_version must be derived from invocation stdout artifact",
    );

    let doc_only_claim = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["claim_source"] = json!("documentation");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(&doc_only_claim, "docs-only claims");

    let schema_drift = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["result"].as_object_mut().unwrap().remove("blind_spots");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(&schema_drift, "result.blind_spots");

    let trusted_activation = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        expected["capability_instances"][0]["trusted_adapter_enabled"] = json!(true);
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &trusted_activation,
        "must not enable trusted adapters in phase 1",
    );
}

#[test]
fn host_capability_experiment_uses_canonical_capability_instance_parser() {
    let mut expected = read_json(
        &repo_root().join(FIXTURE_ROOT),
        "expected/normalized-manifest.json",
    );
    for entry in expected["capability_instances"].as_array().unwrap() {
        planr::evidence::parse_verification_capability_instance(
            entry["capability_instance"].clone(),
        )
        .unwrap();
    }
    let entry = expected_entry_mut(&mut expected, "exp-terminal");
    let instance = entry["capability_instance"].clone();

    let mut invalid_environment = instance.clone();
    invalid_environment["environment"] = json!({
        "os": "macos",
        "runtime": "codex"
    });
    assert!(planr::evidence::parse_verification_capability_instance(invalid_environment).is_err());

    let mut invalid_permissions = instance;
    invalid_permissions["permissions"]["approval"] = json!("not_requested");
    assert!(planr::evidence::parse_verification_capability_instance(invalid_permissions).is_err());
}

#[test]
fn host_capability_experiment_rejects_canonical_projection_drift() {
    let invalid_id = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["id"] = json!("INVALID ID");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(&invalid_id, "canonical Rust capability validation failed");

    let invalid_environment = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["environment"] = json!({
            "os": "macos",
            "runtime": "codex"
        });
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &invalid_environment,
        "capability_instance.environment.os is not part of the v1 contract",
    );

    let invalid_permissions = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["permissions"]["approval"] = json!("not_requested");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &invalid_permissions,
        "capability_instance.permissions.approval is not part of the v1 contract",
    );

    let raw_binding_drift = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["permissions"]["filesystem"] = json!("broader-workspace");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &raw_binding_drift,
        "capability_instance.permissions must be projected from raw result permissions",
    );
}

#[test]
fn host_capability_experiment_rejects_manifest_and_digest_forgery() {
    let placeholder_digest = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["manifest_digest"] =
            json!("sha256:1111111111111111111111111111111111111111111111111111111111111111");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &placeholder_digest,
        "capability_instance.manifest_digest must be content-bound by manifest_ref",
    );

    let unbound_raw_digest = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-chrome-browser-client.json");
        raw["events"][1]["payload"]["screenshot_digest"] =
            json!("sha256:983858684170dd40ce4a604a9d0e0863fe9f4f87bd2bc1c40f5c78d397654220");
        write_json(root, "observed/exp-chrome-browser-client.json", &raw);
    });
    assert_replay_rejects(&unbound_raw_digest, "is not part of the v1 contract");

    let nested_artifact_digest = mutated_fixture(|root| {
        let artifact = root.join("artifacts/chrome-browser-client/cdp-result.json");
        fs::write(
            &artifact,
            "{\"nested\":\"sha256:983858684170dd40ce4a604a9d0e0863fe9f4f87bd2bc1c40f5c78d397654220\"}\n",
        )
        .unwrap();
        let mut raw = read_json(root, "observed/exp-chrome-browser-client.json");
        let digest = sha256_file(&artifact);
        raw["result"]["artifact_refs"][0]["digest"] = json!(digest);
        raw["result"]["artifact_digests"]["artifact-chrome-browser-client-cdp-result"] =
            json!(digest);
        write_json(root, "observed/exp-chrome-browser-client.json", &raw);
    });
    assert_replay_rejects(
        &nested_artifact_digest,
        "contains a nested digest claim without an artifact_ref",
    );

    for reason in [
        "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot digest observations",
        "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot SHA-256 observation",
        "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot hash observation",
        "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot checksum observation",
    ] {
        let unsupported_screenshot_claim = mutated_fixture(|root| {
            set_chrome_availability_reason(root, reason);
        });
        assert_replay_rejects(
            &unsupported_screenshot_claim,
            "claims screenshot digest observations without a content-bound screenshot artifact",
        );
    }

    let incomplete_image_screenshot_claims = vec![
        (
            "png-header-only",
            b"\x89PNG\r\n\x1a\n".to_vec(),
            "screenshot digest observations",
        ),
        (
            "png-truncated-ihdr",
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec(),
            "screenshot hash observation",
        ),
        (
            "jpeg-prefix-only",
            b"\xff\xd8\xff".to_vec(),
            "screenshot checksum observation",
        ),
        (
            "jpeg-soi-eoi-no-frame",
            b"\xff\xd8\xff\xd9".to_vec(),
            "screenshot SHA-256 observation",
        ),
        (
            "webp-header-only",
            b"RIFF\x04\x00\x00\x00WEBP".to_vec(),
            "screenshot digest observations",
        ),
        (
            "webp-truncated-vp8",
            b"RIFF\x12\x00\x00\x00WEBPVP8 \x0a\x00\x00\x00\x9d\x01".to_vec(),
            "screenshot hash observation",
        ),
    ];
    for (name, bytes, claim) in incomplete_image_screenshot_claims {
        let incomplete_image_screenshot_claim = mutated_fixture(|root| {
            add_chrome_screenshot_artifact(
                root,
                "screenshot",
                &format!("artifacts/chrome-browser-client/{name}.bin"),
                &bytes,
            );
            set_chrome_availability_reason(
                root,
                &format!(
                    "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and {claim}"
                ),
            );
        });
        assert_replay_rejects(
            &incomplete_image_screenshot_claim,
            "screenshot artifact artifact-chrome-browser-client-screenshot canonical Rust screenshot validation failed",
        );
    }

    let valid_minimal_screenshots = vec![
        ("png", minimal_png_bytes()),
        (
            "jpeg",
            hex_bytes(
                "ffd8ffe000104a46494600010100000100010000ffdb004300080606070605080707070909080a0c140d0c0b0b0c1912130f141d1a1f1e1d1a1c1c20242e2720222c231c1c2837292c30313434341f27393d38323c2e333432ffdb0043010909090c0b0c180d0d1832211c213232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232323232ffc00011080001000103012200021101031101ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b5100002010303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffc4001f0100030101010101010101010000000000000102030405060708090a0bffc400b51100020102040403040705040400010277000102031104052131061241510761711322328108144291a1b1c109233352f0156272d10a162434e125f11718191a262728292a35363738393a434445464748494a535455565758595a636465666768696a737475767778797a82838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae2e3e4e5e6e7e8e9eaf2f3f4f5f6f7f8f9faffda000c03010002110311003f00e2e8a28af993f713ffd9",
            ),
        ),
        (
            "webp",
            hex_bytes(
                "524946463c000000574542505650382030000000d001009d012a0100010001402625a00274ba01f80003b000fef2eb7ffcd815cd73eff7ffd2e0fd2e0fd2e0ffd2900000",
            ),
        ),
    ];
    for (format, bytes) in valid_minimal_screenshots {
        let valid_screenshot_claim = mutated_fixture(|root| {
            add_chrome_screenshot_artifact(
                root,
                "screenshot",
                &format!("artifacts/chrome-browser-client/minimal.{format}"),
                &bytes,
            );
            set_chrome_availability_reason(
                root,
                "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot digest observations",
            );
        });
        let output = replay(&valid_screenshot_claim);
        assert!(
            output.status.success(),
            "{format} screenshot replay failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let metadata_only_screenshot_claim = mutated_fixture(|root| {
        add_chrome_screenshot_artifact(
            root,
            "screenshot",
            "artifacts/chrome-browser-client/screenshot.json",
            br#"{"screenshot":{"bytes_available":true}}"#,
        );
        set_chrome_availability_reason(
            root,
            "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot digest observations",
        );
    });
    assert_replay_rejects(
        &metadata_only_screenshot_claim,
        "screenshot artifact artifact-chrome-browser-client-screenshot canonical Rust screenshot validation failed",
    );

    let renamed_metadata_screenshot_claim = mutated_fixture(|root| {
        add_chrome_screenshot_artifact(
            root,
            "screenshot-json",
            "artifacts/chrome-browser-client/renamed-screenshot.json",
            br#"{"screenshot":{"bytes_available":true}}"#,
        );
        set_chrome_availability_reason(
            root,
            "native Chrome browser-client produced CDP Runtime.evaluate, DOM, and screenshot hash observation",
        );
    });
    assert_replay_rejects(
        &renamed_metadata_screenshot_claim,
        "screenshot artifact artifact-chrome-browser-client-screenshot kind must be screenshot",
    );
}

#[test]
fn host_capability_experiment_rejects_raw_contract_drift() {
    let final_not_last = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["events"].as_array_mut().unwrap().push(json!({
            "sequence": 3,
            "event_name": "after-final",
            "payload_version": "host-event/1.0.0",
            "tool_name": "functions.exec_command",
            "event_source": "codex.developer_tool.exec_command",
            "payload": {}
        }));
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(&final_not_last, "final event must be the last event");

    let host_mismatch = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["host_identity"]["host"] = json!("node");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(
        &host_mismatch,
        "host_identity.host does not match experiment",
    );

    let input_kind_mismatch = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["input"]["input_kind"] = json!("manual_probe");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(
        &input_kind_mismatch,
        "input.input_kind does not match experiment",
    );

    let unknown_result_field = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["result"]["blind_spots_v2"] = json!([]);
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(
        &unknown_result_field,
        "blind_spots_v2 is not part of the v1 contract",
    );
}

#[test]
fn host_capability_experiment_rejects_artifact_graph_drift() {
    let duplicate_artifact_id = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-local-function.json");
        let digest = raw["result"]["artifact_refs"][0]["digest"].clone();
        raw["result"]["artifact_refs"][0]["id"] = json!("artifact-terminal-stdout");
        raw["result"]["artifact_digests"] = json!({
            "artifact-terminal-stdout": digest
        });
        write_json(root, "observed/exp-local-function.json", &raw);
    });
    assert_replay_rejects(
        &duplicate_artifact_id,
        "duplicate artifact id artifact-terminal-stdout",
    );

    let orphan_artifact = mutated_fixture(|root| {
        fs::write(root.join("artifacts/orphan.txt"), "unreferenced\n").unwrap();
    });
    assert_replay_rejects(
        &orphan_artifact,
        "orphan artifact file artifacts/orphan.txt",
    );

    let root_kind_drift = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-terminal.json");
        raw["result"]["artifact_refs"][0]["root_kind"] = json!("workspace_root");
        write_json(root, "observed/exp-terminal.json", &raw);
    });
    assert_replay_rejects(&root_kind_drift, "root_kind must be fixture_root");
}

#[test]
fn host_capability_experiment_rejects_provenance_and_schema_forgery() {
    let forged_non_available_claim = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-codex-exec");
        entry["claim_source"] = json!("observed_capture");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &forged_non_available_claim,
        "claim_source must match provenance",
    );

    let nonexistent_schema_ref = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        expected["schema_refs"]["raw"]["path"] = json!("schemas/missing.schema.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["observed_payload_contract"]["schema_ref"] =
            json!("schemas/missing.schema.json");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(&nonexistent_schema_ref, "points at a missing file");

    let forged_probe_detail = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-terminal");
        entry["capability_instance"]["probe_result"]["checks"][0]["detail"] =
            json!("forged detail");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &forged_probe_detail,
        "probe_result.checks must be bound to provenance",
    );

    let missing_screenshot_bytes = mutated_fixture(|root| {
        let artifact = root.join("artifacts/chrome-browser-client/cdp-result.json");
        fs::write(
            &artifact,
            "{\"screenshot\":{\"size_bytes\":19417,\"digest\":\"sha256:983858684170dd40ce4a604a9d0e0863fe9f4f87bd2bc1c40f5c78d397654220\"}}\n",
        )
        .unwrap();
        let mut raw = read_json(root, "observed/exp-chrome-browser-client.json");
        let digest = sha256_file(&artifact);
        raw["result"]["artifact_refs"][0]["digest"] = json!(digest);
        raw["result"]["artifact_digests"]["artifact-chrome-browser-client-cdp-result"] =
            json!(digest);
        write_json(root, "observed/exp-chrome-browser-client.json", &raw);
    });
    assert_replay_rejects(
        &missing_screenshot_bytes,
        "contains a nested digest claim without an artifact_ref",
    );
}

#[test]
fn host_capability_experiment_executes_strict_json_schemas() {
    let permissive_schema_swap = mutated_fixture(|root| {
        write_json(
            root,
            "schemas/host-capability-expected-manifest.schema.json",
            &json!({}),
        );
        refresh_expected_schema_digest(
            root,
            "expected",
            "schemas/host-capability-expected-manifest.schema.json",
        );
    });
    assert_replay_rejects(
        &permissive_schema_swap,
        "schemas.expected must not be a permissive empty schema",
    );

    let schema_instance_mismatch = mutated_fixture(|root| {
        let mut schema = read_json(root, "schemas/host-capability-observed-raw.schema.json");
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("must_not_exist"));
        schema["properties"]["must_not_exist"] = json!({
            "type": "string",
            "minLength": 1
        });
        write_json(
            root,
            "schemas/host-capability-observed-raw.schema.json",
            &schema,
        );
        refresh_expected_schema_digest(
            root,
            "raw",
            "schemas/host-capability-observed-raw.schema.json",
        );
    });
    assert_replay_rejects(&schema_instance_mismatch, "failed Draft 2020-12 validation");
}

#[test]
fn host_capability_experiment_rejects_claim_source_state_drift() {
    let unprobed_as_mechanical = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-codex-app-server");
        entry["claim_source"] = json!("mechanical_unavailable_probe");
        write_json(root, "expected/normalized-manifest.json", &expected);
        let mut provenance = read_json(root, "provenance/host-capability-captures.json");
        let capture = provenance["captures"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|capture| capture["experiment_id"] == "exp-codex-app-server")
            .unwrap();
        capture["claim_source"] = json!("mechanical_unavailable_probe");
        capture["source_kind"] = json!("mechanical_unavailable_probe");
        capture["observation_mode"] = json!("mechanical_invocation");
        write_json(
            root,
            "provenance/host-capability-captures.json",
            &provenance,
        );
        refresh_provenance_digest(root);
    });
    assert_replay_rejects(
        &unprobed_as_mechanical,
        "claim_source is not valid for raw input_kind",
    );

    let pseudo_version_without_missing_sentinel = mutated_fixture(|root| {
        let mut raw = read_json(root, "observed/exp-codex-exec.json");
        raw["host_identity"]["version"] = json!("unavailable-in-fixture-replay");
        raw["result"]["missing_fields"] = json!(["exec_event_stream", "artifact_contract"]);
        write_json(root, "observed/exp-codex-exec.json", &raw);
        let mut provenance = read_json(root, "provenance/host-capability-captures.json");
        let capture = provenance["captures"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|capture| capture["experiment_id"] == "exp-codex-exec")
            .unwrap();
        capture["host_version"] = json!("unavailable-in-fixture-replay");
        capture["missing_fields"] = json!(["exec_event_stream", "artifact_contract"]);
        write_json(
            root,
            "provenance/host-capability-captures.json",
            &provenance,
        );
        refresh_provenance_digest(root);
    });
    assert_replay_rejects(
        &pseudo_version_without_missing_sentinel,
        "host_version must be actual or explicitly missing",
    );
}

#[test]
fn host_capability_experiment_requires_non_available_reason_binding() {
    let missing_reason = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-codex-mcp-server");
        entry["capability_instance"]["availability"]
            .as_object_mut()
            .unwrap()
            .remove("reason");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &missing_reason,
        "availability.reason is required for non-available statuses",
    );

    let forged_reason = mutated_fixture(|root| {
        let mut expected = read_json(root, "expected/normalized-manifest.json");
        let entry = expected_entry_mut(&mut expected, "exp-codex-mcp-server");
        entry["capability_instance"]["availability"]["reason"] = json!("forged reason");
        write_json(root, "expected/normalized-manifest.json", &expected);
    });
    assert_replay_rejects(
        &forged_reason,
        "availability.reason must be bound to raw and provenance",
    );
}

#[test]
fn host_capability_experiment_validator_boundary_fails_closed() {
    let fixture_root = repo_root().join(FIXTURE_ROOT);
    let script =
        fs::read_to_string(repo_root().join("scripts/host-capability-experiment.mjs")).unwrap();
    let packaging_contract = read_json(&fixture_root, "packaging/validator-resolution.json");
    assert_eq!(
        packaging_contract["default_resolution"]["kind"],
        "packaged_sibling"
    );
    assert_eq!(
        packaging_contract["configured_resolution"]["path_requirement"],
        "absolute built binary path"
    );
    assert!(
        !script.contains("cargo run") && !script.contains("--bin planr-host-capability-validator"),
        "runtime validator resolution must not fall back to cargo"
    );
    assert!(
        !script.contains("target\", \"debug")
            && !script.contains("process.cwd(), validatorExecutableName")
            && !script.contains("PLANR_HOST_CAPABILITY_VALIDATOR_COMPANION_DIR"),
        "runtime validator resolution must not search development fallback paths"
    );

    let source_checkout_replay = replay_without_validator_env(&fixture_root);
    assert!(
        source_checkout_replay.status.success(),
        "source checkout replay with env unset failed: {}",
        String::from_utf8_lossy(&source_checkout_replay.stderr)
    );
    let source_summary: Value = serde_json::from_slice(&source_checkout_replay.stdout).unwrap();
    assert_eq!(source_summary["verdict"], "pass");

    let package_dir = tempdir().unwrap();
    let production_like_default = replay_with_packaged_layout(&fixture_root, package_dir.path());
    assert!(
        production_like_default.status.success(),
        "extracted package companion validator resolution failed: {}",
        String::from_utf8_lossy(&production_like_default.stderr)
    );
    let package_summary: Value = serde_json::from_slice(&production_like_default.stdout).unwrap();
    assert_eq!(package_summary["verdict"], "pass");

    let release_script = fs::read_to_string(repo_root().join("scripts/build-release.sh")).unwrap();
    for expected in [
        "scripts/planr-host-capability-validator",
        "scripts/host-capability-experiment.mjs",
        "scripts/host-capability-runtime",
    ] {
        assert!(
            release_script.contains(expected),
            "release script must copy/checksum {expected}"
        );
    }
    assert!(
        release_script
            .contains("tar -czf \"../$asset\" planr scripts README.md LICENSE.md SHA256SUMS"),
        "release tarball must include runtime scripts without test fixtures"
    );

    let missing_default_dir = tempdir().unwrap();
    let missing_scripts_dir = missing_default_dir.path().join("scripts");
    fs::create_dir_all(&missing_scripts_dir).unwrap();
    fs::copy(
        repo_root().join("scripts/host-capability-experiment.mjs"),
        missing_scripts_dir.join("host-capability-experiment.mjs"),
    )
    .unwrap();
    let missing_default = Command::new("node")
        .current_dir(repo_root())
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .args([
            missing_scripts_dir
                .join("host-capability-experiment.mjs")
                .to_str()
                .unwrap(),
            "replay",
            "--fixture-root",
            fixture_root.to_str().unwrap(),
        ])
        .output()
        .expect("missing packaged validator replay script must run");
    assert!(
        !missing_default.status.success(),
        "missing default validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&missing_default.stderr)
            .contains("planr-host-capability-validator binary was not found")
            || String::from_utf8_lossy(&missing_default.stderr)
                .contains("canonical Rust capability validator not found at packaged sibling"),
        "{}",
        String::from_utf8_lossy(&missing_default.stderr)
    );

    let missing_validator = replay_with_env(
        &fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            "/planr/definitely/missing/validator",
        )],
    );
    assert!(
        !missing_validator.status.success(),
        "missing validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&missing_validator.stderr)
            .contains("validator path points at a missing file"),
        "{}",
        String::from_utf8_lossy(&missing_validator.stderr)
    );

    let true_validator = replay_with_env(
        &fixture_root,
        &[("PLANR_HOST_CAPABILITY_VALIDATOR", "/usr/bin/true")],
    );
    assert!(
        !true_validator.status.success(),
        "zero-output validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&true_validator.stderr)
            .contains("validator identity produced empty stdout"),
        "{}",
        String::from_utf8_lossy(&true_validator.stderr)
    );

    let nonzero_validator = replay_with_env(
        &fixture_root,
        &[("PLANR_HOST_CAPABILITY_VALIDATOR", "/usr/bin/false")],
    );
    assert!(
        !nonzero_validator.status.success(),
        "nonzero validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&nonzero_validator.stderr)
            .contains("canonical Rust capability validator identity probe failed"),
        "{}",
        String::from_utf8_lossy(&nonzero_validator.stderr)
    );

    let dir = tempdir().unwrap();
    let wrong_counts_validator = dir.path().join("wrong-counts.sh");
    write_executable(
        &wrong_counts_validator,
        "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}'\n  exit 0\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{print $1}')\nprintf '%s\\n' \"{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":0,\\\"validated_instances\\\":0}\"\n",
    );
    let wrong_counts = replay_with_env(
        &fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            wrong_counts_validator.to_str().unwrap(),
        )],
    );
    assert!(
        !wrong_counts.status.success(),
        "wrong-counts validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&wrong_counts.stderr).contains(
            "validator result.validated_raw_documents must match submitted raw document count"
        ),
        "{}",
        String::from_utf8_lossy(&wrong_counts.stderr)
    );

    let forged_validators = [
        (
            "wrong-digest.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:1111111111111111111111111111111111111111111111111111111111111111","validated_raw_documents":14,"validated_instances":14}"#,
            "validator result.input_digest must match submitted validation bundle",
        ),
        (
            "wrong-version.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v2","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":14,"validated_instances":14}"#,
            "validator result.schema_version must be planr.host_capability_validator_result.v1",
        ),
        (
            "extra-field.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":14,"validated_instances":14,"extra":true}"#,
            "validator result.extra is not part of the v1 contract",
        ),
        (
            "non-pass.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"fail","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":14,"validated_instances":14}"#,
            "validator result.verdict must be pass",
        ),
    ];
    for (name, result, expected) in forged_validators {
        let validator = dir.path().join(name);
        write_executable(
            &validator,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nprintf '%s\\n' '{result}'\n"
            ),
        );
        let output = replay_with_env(
            &fixture_root,
            &[(
                "PLANR_HOST_CAPABILITY_VALIDATOR",
                validator.to_str().unwrap(),
            )],
        );
        assert!(
            !output.status.success(),
            "forged validator {name} unexpectedly passed"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "stderr for {name} did not contain {expected:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let malformed_validator = dir.path().join("malformed.sh");
    write_executable(
        &malformed_validator,
        "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}'\n  exit 0\nfi\nprintf '%s\\n' 'not-json'\n",
    );
    let malformed = replay_with_env(
        &fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            malformed_validator.to_str().unwrap(),
        )],
    );
    assert!(
        !malformed.status.success(),
        "malformed validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&malformed.stderr)
            .contains("validator result produced malformed JSON"),
        "{}",
        String::from_utf8_lossy(&malformed.stderr)
    );

    let stderr_validator = dir.path().join("stderr.sh");
    write_executable(
        &stderr_validator,
        "#!/bin/sh\nprintf '%s\\n' 'diagnostic' >&2\nprintf '%s\\n' '{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}'\n",
    );
    let stderr_output = replay_with_env(
        &fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            stderr_validator.to_str().unwrap(),
        )],
    );
    assert!(
        !stderr_output.status.success(),
        "stderr-writing validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&stderr_output.stderr)
            .contains("canonical Rust capability validator identity probe wrote stderr"),
        "{}",
        String::from_utf8_lossy(&stderr_output.stderr)
    );

    let timeout_pid_file = dir.path().join("timeout-child.pid");
    let timeout_tree_validator = dir.path().join("timeout-tree.sh");
    write_executable(
        &timeout_tree_validator,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nsleep 60 &\nprintf '%s\\n' \"$!\" > '{}'\nsleep 60\n",
            timeout_pid_file.display()
        ),
    );
    let timeout_started = Instant::now();
    let timeout_tree = replay_with_env(
        &fixture_root,
        &[
            (
                "PLANR_HOST_CAPABILITY_VALIDATOR",
                timeout_tree_validator.to_str().unwrap(),
            ),
            ("PLANR_HOST_CAPABILITY_VALIDATOR_TIMEOUT_MS", "100"),
        ],
    );
    let timeout_elapsed = timeout_started.elapsed();
    assert!(
        !timeout_tree.status.success(),
        "timeout tree validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&timeout_tree.stderr)
            .contains("canonical Rust capability validation failed to execute: timed out"),
        "{}",
        String::from_utf8_lossy(&timeout_tree.stderr)
    );
    assert!(
        timeout_elapsed < Duration::from_secs(5),
        "timeout tree cleanup took {timeout_elapsed:?}"
    );
    let timeout_child_pid = read_pid(&timeout_pid_file);
    assert!(
        wait_process_gone(timeout_child_pid),
        "timeout descendant process {timeout_child_pid} was not cleaned up"
    );

    let nonzero_pid_file = dir.path().join("nonzero-child.pid");
    let nonzero_tree_validator = dir.path().join("nonzero-tree.sh");
    write_executable(
        &nonzero_tree_validator,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nsleep 60 &\nprintf '%s\\n' \"$!\" > '{}'\nexit 7\n",
            nonzero_pid_file.display()
        ),
    );
    let nonzero_started = Instant::now();
    let nonzero_tree = replay_with_env(
        &fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            nonzero_tree_validator.to_str().unwrap(),
        )],
    );
    let nonzero_elapsed = nonzero_started.elapsed();
    assert!(
        !nonzero_tree.status.success(),
        "nonzero tree validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&nonzero_tree.stderr).contains("status 7"),
        "{}",
        String::from_utf8_lossy(&nonzero_tree.stderr)
    );
    assert!(
        nonzero_elapsed < Duration::from_secs(5),
        "nonzero tree cleanup took {nonzero_elapsed:?}"
    );
    let nonzero_child_pid = read_pid(&nonzero_pid_file);
    assert!(
        wait_process_gone(nonzero_child_pid),
        "nonzero descendant process {nonzero_child_pid} was not cleaned up"
    );

    let success_pid_file = dir.path().join("success-child.pid");
    let success_tree_validator = dir.path().join("success-tree.sh");
    write_executable(
        &success_tree_validator,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nsleep 60 &\nprintf '%s\\n' \"$!\" > '{}'\ndigest=$(shasum -a 256 \"$2\" | awk '{{print $1}}')\nprintf '%s\\n' \"{{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}}\"\n",
            success_pid_file.display()
        ),
    );
    let success_started = Instant::now();
    let success_tree = replay_with_env(
        &fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            success_tree_validator.to_str().unwrap(),
        )],
    );
    let success_elapsed = success_started.elapsed();
    assert!(
        success_tree.status.success(),
        "success tree validator failed: {}",
        String::from_utf8_lossy(&success_tree.stderr)
    );
    assert!(
        success_elapsed < Duration::from_secs(5),
        "success tree cleanup took {success_elapsed:?}"
    );
    let success_child_pid = read_pid(&success_pid_file);
    assert!(
        wait_process_gone(success_child_pid),
        "success descendant process {success_child_pid} was not cleaned up"
    );

    let screenshot_fixture_root = valid_screenshot_claim_fixture();
    let screenshot_validators = [
        (
            "screenshot-empty.sh",
            "",
            "screenshot validator result produced empty stdout",
        ),
        (
            "screenshot-malformed.sh",
            "not-json",
            "screenshot validator result produced malformed JSON",
        ),
        (
            "screenshot-wrong-digest.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:1111111111111111111111111111111111111111111111111111111111111111","validated_raw_documents":0,"validated_instances":0}"#,
            "validator result.input_digest must match submitted validation bundle",
        ),
        (
            "screenshot-wrong-version.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v2","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":0,"validated_instances":0}"#,
            "validator result.schema_version must be planr.host_capability_validator_result.v1",
        ),
        (
            "screenshot-wrong-validator.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"forged-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":0,"validated_instances":0}"#,
            "validator result.validator must be planr-host-capability-validator",
        ),
        (
            "screenshot-extra-field.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"pass","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":0,"validated_instances":0,"extra":true}"#,
            "validator result.extra is not part of the v1 contract",
        ),
        (
            "screenshot-non-pass.sh",
            r#"{"schema_version":"planr.host_capability_validator_result.v1","validator":"planr-host-capability-validator","validator_version":"1.0.0","verdict":"fail","input_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","validated_raw_documents":0,"validated_instances":0}"#,
            "validator result.verdict must be pass",
        ),
    ];
    for (name, screenshot_result, expected) in screenshot_validators {
        let validator = dir.path().join(name);
        write_executable(
            &validator,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nif [ \"$1\" = \"--validate-screenshot\" ]; then\n  printf '%s\\n' '{screenshot_result}'\n  exit 0\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{{print $1}}')\nprintf '%s\\n' \"{{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}}\"\n"
            ),
        );
        let output = replay_with_env(
            &screenshot_fixture_root,
            &[(
                "PLANR_HOST_CAPABILITY_VALIDATOR",
                validator.to_str().unwrap(),
            )],
        );
        assert!(
            !output.status.success(),
            "forged screenshot validator {name} unexpectedly passed"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "stderr for {name} did not contain {expected:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let screenshot_wrong_counts_validator = dir.path().join("screenshot-wrong-counts.sh");
    write_executable(
        &screenshot_wrong_counts_validator,
        "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}'\n  exit 0\nfi\nif [ \"$1\" = \"--validate-screenshot\" ]; then\n  digest=$(shasum -a 256 \"$2\" | awk '{print $1}')\n  printf '%s\\n' \"{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}\"\n  exit 0\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{print $1}')\nprintf '%s\\n' \"{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}\"\n",
    );
    let screenshot_wrong_counts = replay_with_env(
        &screenshot_fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            screenshot_wrong_counts_validator.to_str().unwrap(),
        )],
    );
    assert!(
        !screenshot_wrong_counts.status.success(),
        "wrong-counts screenshot validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&screenshot_wrong_counts.stderr).contains(
            "validator result.validated_raw_documents must match submitted raw document count"
        ),
        "{}",
        String::from_utf8_lossy(&screenshot_wrong_counts.stderr)
    );

    let screenshot_stderr_validator = dir.path().join("screenshot-stderr.sh");
    write_executable(
        &screenshot_stderr_validator,
        "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}'\n  exit 0\nfi\nif [ \"$1\" = \"--validate-screenshot\" ]; then\n  printf '%s\\n' 'diagnostic' >&2\n  digest=$(shasum -a 256 \"$2\" | awk '{print $1}')\n  printf '%s\\n' \"{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":0,\\\"validated_instances\\\":0}\"\n  exit 0\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{print $1}')\nprintf '%s\\n' \"{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}\"\n",
    );
    let screenshot_stderr = replay_with_env(
        &screenshot_fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            screenshot_stderr_validator.to_str().unwrap(),
        )],
    );
    assert!(
        !screenshot_stderr.status.success(),
        "stderr-writing screenshot validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&screenshot_stderr.stderr)
            .contains("canonical Rust screenshot validation wrote stderr"),
        "{}",
        String::from_utf8_lossy(&screenshot_stderr.stderr)
    );

    let screenshot_nonzero_pid_file = dir.path().join("screenshot-nonzero-child.pid");
    let screenshot_nonzero_tree_validator = dir.path().join("screenshot-nonzero-tree.sh");
    write_executable(
        &screenshot_nonzero_tree_validator,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nif [ \"$1\" = \"--validate-screenshot\" ]; then\n  sleep 60 &\n  printf '%s\\n' \"$!\" > '{}'\n  exit 7\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{{print $1}}')\nprintf '%s\\n' \"{{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}}\"\n",
            screenshot_nonzero_pid_file.display()
        ),
    );
    let screenshot_nonzero = replay_with_env(
        &screenshot_fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            screenshot_nonzero_tree_validator.to_str().unwrap(),
        )],
    );
    assert!(
        !screenshot_nonzero.status.success(),
        "nonzero screenshot validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&screenshot_nonzero.stderr).contains("status 7"),
        "{}",
        String::from_utf8_lossy(&screenshot_nonzero.stderr)
    );
    let screenshot_nonzero_child_pid = read_pid(&screenshot_nonzero_pid_file);
    assert!(
        wait_process_gone(screenshot_nonzero_child_pid),
        "nonzero screenshot descendant process {screenshot_nonzero_child_pid} was not cleaned up"
    );

    let screenshot_timeout_pid_file = dir.path().join("screenshot-timeout-child.pid");
    let screenshot_timeout_tree_validator = dir.path().join("screenshot-timeout-tree.sh");
    write_executable(
        &screenshot_timeout_tree_validator,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nif [ \"$1\" = \"--validate-screenshot\" ]; then\n  sleep 60 &\n  printf '%s\\n' \"$!\" > '{}'\n  sleep 60\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{{print $1}}')\nprintf '%s\\n' \"{{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}}\"\n",
            screenshot_timeout_pid_file.display()
        ),
    );
    let screenshot_timeout = replay_with_env(
        &screenshot_fixture_root,
        &[
            (
                "PLANR_HOST_CAPABILITY_VALIDATOR",
                screenshot_timeout_tree_validator.to_str().unwrap(),
            ),
            ("PLANR_HOST_CAPABILITY_VALIDATOR_TIMEOUT_MS", "100"),
        ],
    );
    assert!(
        !screenshot_timeout.status.success(),
        "timeout screenshot validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&screenshot_timeout.stderr)
            .contains("canonical Rust screenshot validation failed to execute: timed out"),
        "{}",
        String::from_utf8_lossy(&screenshot_timeout.stderr)
    );
    let screenshot_timeout_child_pid = read_pid(&screenshot_timeout_pid_file);
    assert!(
        wait_process_gone(screenshot_timeout_child_pid),
        "timeout screenshot descendant process {screenshot_timeout_child_pid} was not cleaned up"
    );

    let screenshot_success_pid_file = dir.path().join("screenshot-success-child.pid");
    let screenshot_success_tree_validator = dir.path().join("screenshot-success-tree.sh");
    write_executable(
        &screenshot_success_tree_validator,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--identity\" ]; then\n  printf '%s\\n' '{{\"schema_version\":\"planr.host_capability_validator_identity.v1\",\"validator\":\"planr-host-capability-validator\",\"validator_version\":\"1.0.0\"}}'\n  exit 0\nfi\nif [ \"$1\" = \"--validate-screenshot\" ]; then\n  sleep 60 &\n  printf '%s\\n' \"$!\" > '{}'\n  digest=$(shasum -a 256 \"$2\" | awk '{{print $1}}')\n  printf '%s\\n' \"{{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":0,\\\"validated_instances\\\":0}}\"\n  exit 0\nfi\ndigest=$(shasum -a 256 \"$2\" | awk '{{print $1}}')\nprintf '%s\\n' \"{{\\\"schema_version\\\":\\\"planr.host_capability_validator_result.v1\\\",\\\"validator\\\":\\\"planr-host-capability-validator\\\",\\\"validator_version\\\":\\\"1.0.0\\\",\\\"verdict\\\":\\\"pass\\\",\\\"input_digest\\\":\\\"sha256:$digest\\\",\\\"validated_raw_documents\\\":14,\\\"validated_instances\\\":14}}\"\n",
            screenshot_success_pid_file.display()
        ),
    );
    let screenshot_success = replay_with_env(
        &screenshot_fixture_root,
        &[(
            "PLANR_HOST_CAPABILITY_VALIDATOR",
            screenshot_success_tree_validator.to_str().unwrap(),
        )],
    );
    assert!(
        screenshot_success.status.success(),
        "success screenshot validator failed: {}",
        String::from_utf8_lossy(&screenshot_success.stderr)
    );
    let screenshot_success_child_pid = read_pid(&screenshot_success_pid_file);
    assert!(
        wait_process_gone(screenshot_success_child_pid),
        "success screenshot descendant process {screenshot_success_child_pid} was not cleaned up"
    );

    let validator = dir.path().join("sleep-validator.sh");
    write_executable(&validator, "#!/bin/sh\nsleep 5\n");
    let timeout_validator = replay_with_env(
        &fixture_root,
        &[
            (
                "PLANR_HOST_CAPABILITY_VALIDATOR",
                validator.to_str().unwrap(),
            ),
            ("PLANR_HOST_CAPABILITY_VALIDATOR_TIMEOUT_MS", "50"),
        ],
    );
    assert!(
        !timeout_validator.status.success(),
        "timeout validator unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&timeout_validator.stderr)
            .contains("canonical Rust capability validator identity probe failed to execute"),
        "{}",
        String::from_utf8_lossy(&timeout_validator.stderr)
    );
}

#[test]
fn host_capability_source_documentation_examples_capture_import_and_replay() {
    run_host_capability_doc_smoke(&repo_root());
}

#[test]
fn host_capability_release_archive_replays_without_validator_override() {
    let dist = tempdir().unwrap();
    let output = Command::new("sh")
        .current_dir(repo_root())
        .env("PLANR_DIST_DIR", dist.path())
        .env("PLANR_TARGET", native_target())
        .arg("scripts/build-release.sh")
        .output()
        .expect("build-release script must run");
    assert!(
        output.status.success(),
        "build-release failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let archive = dist
        .path()
        .join(format!("planr-{}.tar.gz", native_target()));
    assert!(archive.exists(), "release archive missing: {archive:?}");
    let extract = tempdir().unwrap();
    let tar = Command::new("tar")
        .args([
            "-xzf",
            archive.to_str().unwrap(),
            "-C",
            extract.path().to_str().unwrap(),
        ])
        .output()
        .expect("release archive must extract");
    assert!(
        tar.status.success(),
        "tar extract failed: {}",
        String::from_utf8_lossy(&tar.stderr)
    );
    let checksum = Command::new("sh")
        .current_dir(extract.path())
        .arg("-c")
        .arg("shasum -a 256 -c SHA256SUMS")
        .output()
        .expect("release checksums must run");
    assert!(
        checksum.status.success(),
        "release checksums failed: {}",
        String::from_utf8_lossy(&checksum.stderr)
    );
    let capture = tempdir().unwrap();
    let capture_out = fs::canonicalize(capture.path())
        .unwrap()
        .join("host-capability-capture");
    let capture_output = Command::new("node")
        .current_dir(extract.path())
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .args([
            "scripts/host-capability-experiment.mjs",
            "capture",
            "--out-dir",
            capture_out.to_str().unwrap(),
        ])
        .output()
        .expect("extracted release capture must run");
    assert!(
        capture_output.status.success(),
        "extracted release capture failed: {}",
        String::from_utf8_lossy(&capture_output.stderr)
    );
    assert!(
        capture_out
            .join("expected/normalized-manifest.json")
            .exists(),
        "extracted release capture must write replayable expected manifest"
    );
}

#[test]
fn host_capability_npm_package_replays_with_native_validator_bytes() {
    let fixture = tempdir().unwrap();
    let package = fixture.path();
    fs::copy(
        repo_root().join("package.json"),
        package.join("package.json"),
    )
    .unwrap();
    fs::copy(repo_root().join("README.md"), package.join("README.md")).unwrap();
    fs::copy(repo_root().join("LICENSE.md"), package.join("LICENSE.md")).unwrap();
    copy_dir(&repo_root().join("npm"), &package.join("npm"));
    copy_dir(&repo_root().join("scripts"), &package.join("scripts"));
    let native_dir = package.join("npm/native").join(native_target());
    fs::create_dir_all(&native_dir).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_planr"), native_dir.join("planr")).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_planr-host-capability-validator"),
        native_dir.join("planr-host-capability-validator"),
    )
    .unwrap();
    make_executable(&native_dir.join("planr"));
    make_executable(&native_dir.join("planr-host-capability-validator"));

    let pack_dir = tempdir().unwrap();
    let pack = Command::new("npm")
        .current_dir(package)
        .args([
            "pack",
            "--pack-destination",
            pack_dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("npm pack must run");
    assert!(
        pack.status.success(),
        "npm pack failed: {}",
        String::from_utf8_lossy(&pack.stderr)
    );
    let pack_json: Value = serde_json::from_slice(&pack.stdout).unwrap();
    let files: BTreeSet<_> = pack_json[0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .collect();
    for expected in [
        format!("npm/native/{}/planr", native_target()),
        format!(
            "npm/native/{}/planr-host-capability-validator",
            native_target()
        ),
        "scripts/host-capability-experiment.mjs".to_string(),
        "scripts/host-capability-runtime/v1/schemas/host-capability-observed-raw.schema.json"
            .to_string(),
        "scripts/host-capability-runtime/v1/schemas/host-capability-expected-manifest.schema.json"
            .to_string(),
        "scripts/host-capability-runtime/v1/schemas/host-capability-provenance.schema.json"
            .to_string(),
    ] {
        assert!(
            files.contains(expected.as_str()),
            "npm pack omitted {expected}"
        );
    }
    assert!(
        files.iter().all(|file| !file.starts_with("tests/")),
        "npm package must not include test fixture paths; files={files:?}"
    );

    let tarball = fs::read_dir(pack_dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("tgz"))
        .unwrap();
    let extract = tempdir().unwrap();
    let tar = Command::new("tar")
        .args([
            "-xzf",
            tarball.to_str().unwrap(),
            "-C",
            extract.path().to_str().unwrap(),
        ])
        .output()
        .expect("npm package must extract");
    assert!(
        tar.status.success(),
        "npm extract failed: {}",
        String::from_utf8_lossy(&tar.stderr)
    );
    let extracted = extract.path().join("package");
    let capture = tempdir().unwrap();
    let capture_out = fs::canonicalize(capture.path())
        .unwrap()
        .join("host-capability-capture");
    let capture_output = Command::new("node")
        .current_dir(&extracted)
        .env_remove("PLANR_HOST_CAPABILITY_VALIDATOR")
        .args([
            "scripts/host-capability-experiment.mjs",
            "capture",
            "--out-dir",
            capture_out.to_str().unwrap(),
        ])
        .output()
        .expect("extracted npm capture must run");
    assert!(
        capture_output.status.success(),
        "extracted npm capture failed: {}",
        String::from_utf8_lossy(&capture_output.stderr)
    );
    assert!(
        capture_out
            .join("expected/normalized-manifest.json")
            .exists(),
        "extracted npm capture must write replayable expected manifest"
    );
}

#[test]
fn host_capability_capture_executes_local_function_and_marks_placeholders_unverified() {
    let dir = tempdir().unwrap();
    let output = capture(dir.path());
    assert!(
        output.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["verdict"], "pass");
    assert_eq!(
        summary["availability"]["exp-chrome-browser-client"],
        json!({
            "availability_status": "unavailable",
            "trusted_adapter_enabled": false
        })
    );
    assert_eq!(
        summary["availability"]["exp-mcp-browser"]["availability_status"],
        "unavailable"
    );
    assert_eq!(
        summary["availability"]["exp-computer-use"]["availability_status"],
        "unavailable"
    );
    let chrome_raw = read_json(dir.path(), "observed/exp-chrome-browser-client.json");
    assert_eq!(chrome_raw["input"]["input_kind"], "unprobed_placeholder");
    assert_eq!(
        chrome_raw["result"]["blind_spots"][0],
        "capture mode emitted an explicit placeholder, not an observed support claim"
    );
    let local_result =
        fs::read_to_string(dir.path().join("artifacts/local-function/result.json")).unwrap();
    assert_eq!(local_result, "{\"result\":\"ok\",\"value\":42}\n");
}

fn assert_peer_probe_downgraded_to_placeholder(root: &Path, experiment_id: &str) {
    let raw = read_json(root, &format!("observed/{experiment_id}.json"));
    assert_eq!(raw["host_identity"]["version"], "missing", "{raw}");
    assert_eq!(raw["input"]["input_kind"], "unprobed_placeholder", "{raw}");
    assert_eq!(raw["input"]["replay_mode"], "unprobed-placeholder");
    assert_eq!(raw["result"]["final_status"], "unavailable");
    assert!(
        raw["result"]["missing_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "host_version"),
        "{raw}"
    );
    assert_eq!(raw["result"]["artifact_refs"], json!([]));
    assert_eq!(raw["result"]["artifact_digests"], json!({}));

    let expected = read_json(root, "expected/normalized-manifest.json");
    let entry = expected["capability_instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["raw_capture_id"] == experiment_id)
        .unwrap();
    assert_eq!(entry["claim_source"], "capture_mode_placeholder");
    assert_eq!(entry["trusted_adapter_enabled"], false);
    assert_eq!(
        entry["capability_instance"]["availability"]["status"],
        "unavailable"
    );

    let provenance = read_json(root, "provenance/host-capability-captures.json");
    let capture = provenance["captures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capture| capture["experiment_id"] == experiment_id)
        .unwrap();
    assert_eq!(capture["claim_source"], "capture_mode_placeholder");
    assert_eq!(capture["source_kind"], "unprobed_placeholder");
    assert_eq!(capture["observation_mode"], "unprobed_placeholder");
    assert_eq!(capture["host_version"], "missing");
    assert_eq!(capture["artifact_ids"], json!([]));
    assert!(
        !root
            .join("artifacts")
            .join(experiment_id.trim_start_matches("exp-"))
            .exists(),
        "failed peer probe temporary/artifact directory leaked"
    );
}

#[test]
fn host_capability_failed_peer_probes_downgrade_in_default_and_import_capture() {
    let tools = tempdir().unwrap();
    let missing = tools.path().join("missing-peer-probe");
    let malformed = tools.path().join("malformed-peer-probe");
    let nonzero = tools.path().join("nonzero-peer-probe");
    let signaled = tools.path().join("signaled-peer-probe");
    write_executable(
        &malformed,
        "#!/bin/sh\nprintf 'not-a-planr-host-version\\n'\n",
    );
    write_executable(
        &nonzero,
        "#!/bin/sh\nprintf 'probe failed\\n' >&2\nexit 23\n",
    );
    write_executable(&signaled, "#!/bin/sh\nkill -TERM $$\n");
    let overrides = [
        ("exp-claude-code-host-capture", missing.as_path()),
        ("exp-cursor-agent-host-capture", malformed.as_path()),
    ];

    let default_root = tempdir().unwrap();
    let default_out = default_root.path().join("capture");
    let default_output = capture_with_peer_probe_commands(&default_out, None, &overrides);
    assert!(
        default_output.status.success(),
        "default capture failed: {}",
        String::from_utf8_lossy(&default_output.stderr)
    );
    assert_peer_probe_downgraded_to_placeholder(&default_out, "exp-claude-code-host-capture");
    assert_peer_probe_downgraded_to_placeholder(&default_out, "exp-cursor-agent-host-capture");
    assert!(replay(&default_out).status.success());

    let import_root = observed_bundle(|_| {});
    let imported_root = tempdir().unwrap();
    let imported_out = imported_root.path().join("capture");
    let imported_output =
        capture_with_peer_probe_commands(&imported_out, Some(&import_root), &overrides);
    assert!(
        imported_output.status.success(),
        "import capture failed: {}",
        String::from_utf8_lossy(&imported_output.stderr)
    );
    assert_peer_probe_downgraded_to_placeholder(&imported_out, "exp-claude-code-host-capture");
    assert_peer_probe_downgraded_to_placeholder(&imported_out, "exp-cursor-agent-host-capture");
    assert!(replay(&imported_out).status.success());

    let exit_overrides = [
        ("exp-claude-code-host-capture", nonzero.as_path()),
        ("exp-cursor-agent-host-capture", signaled.as_path()),
    ];
    let exit_default_root = tempdir().unwrap();
    let exit_default_out = exit_default_root.path().join("capture");
    let exit_default_output =
        capture_with_peer_probe_commands(&exit_default_out, None, &exit_overrides);
    assert!(
        exit_default_output.status.success(),
        "nonzero/signal default capture failed: {}",
        String::from_utf8_lossy(&exit_default_output.stderr)
    );
    assert_peer_probe_downgraded_to_placeholder(&exit_default_out, "exp-claude-code-host-capture");
    assert_peer_probe_downgraded_to_placeholder(&exit_default_out, "exp-cursor-agent-host-capture");
    assert!(replay(&exit_default_out).status.success());

    let exit_import_root = observed_bundle(|_| {});
    let exit_imported_root = tempdir().unwrap();
    let exit_imported_out = exit_imported_root.path().join("capture");
    let exit_imported_output = capture_with_peer_probe_commands(
        &exit_imported_out,
        Some(&exit_import_root),
        &exit_overrides,
    );
    assert!(
        exit_imported_output.status.success(),
        "nonzero/signal import capture failed: {}",
        String::from_utf8_lossy(&exit_imported_output.stderr)
    );
    assert_peer_probe_downgraded_to_placeholder(&exit_imported_out, "exp-claude-code-host-capture");
    assert_peer_probe_downgraded_to_placeholder(
        &exit_imported_out,
        "exp-cursor-agent-host-capture",
    );
    assert!(replay(&exit_imported_out).status.success());
}

#[test]
fn host_capability_capture_rejects_destructive_output_targets() {
    let repo_output = capture(&repo_root());
    assert!(
        !repo_output.status.success(),
        "repo root capture unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&repo_output.stderr).contains("repository root"),
        "{}",
        String::from_utf8_lossy(&repo_output.stderr)
    );

    let non_empty = tempdir().unwrap();
    let sentinel = non_empty.path().join("sentinel.txt");
    fs::write(&sentinel, "keep me\n").unwrap();
    let output = capture(non_empty.path());
    assert!(
        !output.status.success(),
        "non-empty capture unexpectedly passed"
    );
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep me\n");

    let import_root = observed_bundle(|_| {});
    let same_path = capture_import(&import_root, &import_root);
    assert!(
        !same_path.status.success(),
        "same-path import unexpectedly passed"
    );
    assert!(import_root.join("external-capture-envelope.json").exists());

    let overlap = capture_import(&import_root.join("child"), &import_root);
    assert!(
        !overlap.status.success(),
        "overlap import unexpectedly passed"
    );
    assert!(import_root.join("external-capture-envelope.json").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link_parent = tempdir().unwrap();
        let target = tempdir().unwrap();
        let target_sentinel = target.path().join("sentinel.txt");
        fs::write(&target_sentinel, "keep symlink target\n").unwrap();
        let link = link_parent.path().join("linked-output");
        symlink(target.path(), &link).unwrap();
        let output = capture(&link);
        assert!(
            !output.status.success(),
            "symlink capture unexpectedly passed"
        );
        assert_eq!(
            fs::read_to_string(&target_sentinel).unwrap(),
            "keep symlink target\n"
        );
    }
}

#[test]
fn host_capability_capture_rejects_external_experiment_id_path_attacks_before_write() {
    let outside = tempdir().unwrap();
    let sentinel = outside.path().join("planr-review-import-escape.json");
    fs::write(&sentinel, "outside sentinel\n").unwrap();
    let traversal = format!(
        "../../../../../../../../{}",
        sentinel
            .with_extension("")
            .to_string_lossy()
            .trim_start_matches('/')
    );
    let attacks = [
        traversal.as_str(),
        "/tmp/planr-review-import-absolute",
        "exp-chrome-browser-client/child",
        "exp-chrome-browser-client\\child",
        "exp%2Fchrome-browser-client",
        "exp-chrome-browser-client\u{2215}child",
        "exp-unknown-surface",
    ];
    for attack in attacks {
        let import_root = observed_bundle(|root| {
            mutate_external_capture(root, |raw| {
                raw["experiment_id"] = json!(attack);
            });
        });
        let out = tempdir().unwrap();
        let output = capture_import(out.path(), &import_root);
        assert!(
            !output.status.success(),
            "attack {attack:?} unexpectedly passed"
        );
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "outside sentinel\n");
        assert!(
            !outside
                .path()
                .join("planr-review-import-absolute.json")
                .exists()
        );
    }

    let duplicate = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        let raw = envelope["captures"][0].clone();
        envelope["captures"].as_array_mut().unwrap().push(raw);
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &duplicate);
    assert!(
        !output.status.success(),
        "duplicate external capture unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate external capture"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "outside sentinel\n");
}

#[test]
fn host_capability_capture_rejects_symlink_parents_and_preserves_targets() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let link_holder = tempdir().unwrap();
        let real_parent = tempdir().unwrap();
        let sentinel = real_parent.path().join("sentinel.txt");
        fs::write(&sentinel, "real parent sentinel\n").unwrap();
        let linked_parent = link_holder.path().join("linked-parent");
        symlink(real_parent.path(), &linked_parent).unwrap();
        let output = capture(&linked_parent.join("output"));
        assert!(
            !output.status.success(),
            "symlink parent capture unexpectedly passed"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("symlink component"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(&sentinel).unwrap(),
            "real parent sentinel\n"
        );

        let nested_output = capture(&linked_parent.join("missing").join("output"));
        assert!(
            !nested_output.status.success(),
            "nested symlink parent capture unexpectedly passed"
        );
        assert_eq!(
            fs::read_to_string(&sentinel).unwrap(),
            "real parent sentinel\n"
        );

        let grand_holder = tempdir().unwrap();
        let real_grandparent = tempdir().unwrap();
        let grand_sentinel = real_grandparent.path().join("grand-sentinel.txt");
        fs::write(&grand_sentinel, "grand parent sentinel\n").unwrap();
        let linked_grandparent = grand_holder.path().join("linked-grandparent");
        symlink(real_grandparent.path(), &linked_grandparent).unwrap();
        let grand_output = capture(&linked_grandparent.join("child").join("output"));
        assert!(
            !grand_output.status.success(),
            "grandparent symlink capture unexpectedly passed"
        );
        assert_eq!(
            fs::read_to_string(&grand_sentinel).unwrap(),
            "grand parent sentinel\n"
        );

        let import_real = observed_bundle(|_| {});
        let import_link = link_holder.path().join("linked-import");
        symlink(&import_real, &import_link).unwrap();
        let import_output = capture_import(tempdir().unwrap().path(), &import_link);
        assert!(
            !import_output.status.success(),
            "symlink import root unexpectedly passed"
        );
        assert!(import_real.join("external-capture-envelope.json").exists());
    }
}

#[test]
fn host_capability_capture_imports_observed_connector_fixture() {
    let dir = tempdir().unwrap();
    let import_root = observed_bundle(|_| {});
    assert!(!import_root.join("expected").exists());
    assert!(!import_root.join("provenance").exists());
    assert!(!import_root.join("schemas").exists());
    let output = capture_import(dir.path(), &import_root);
    assert!(
        output.status.success(),
        "capture import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["verdict"], "pass");
    assert_eq!(
        summary["availability"]["exp-chrome-browser-client"],
        json!({
            "availability_status": "available",
            "trusted_adapter_enabled": false
        })
    );
    assert_eq!(
        summary["availability"]["exp-mcp-browser"]["availability_status"],
        "unavailable"
    );
    assert_eq!(
        summary["availability"]["exp-computer-use"]["availability_status"],
        "unavailable"
    );

    let chrome_raw = read_json(dir.path(), "observed/exp-chrome-browser-client.json");
    assert_eq!(
        chrome_raw["tool_name"],
        "browser-client.mjs chrome Runtime.evaluate"
    );
    assert_eq!(
        chrome_raw["provenance_ref"]["path"],
        "provenance/host-capability-captures.json"
    );
    assert_eq!(
        chrome_raw["provenance_ref"]["digest"],
        sha256_file(&dir.path().join("provenance/host-capability-captures.json"))
    );
    assert_eq!(chrome_raw["result"]["final_status"], "available");
    let artifact = &chrome_raw["result"]["artifact_refs"][0];
    let artifact_path = artifact["path"].as_str().unwrap();
    let copied_artifact = dir.path().join(artifact_path);
    assert!(
        copied_artifact.exists(),
        "import must copy bound artifact bytes"
    );
    assert_eq!(artifact["digest"], sha256_file(&copied_artifact));
    let provenance = read_json(dir.path(), "provenance/host-capability-captures.json");
    let chrome_provenance = provenance["captures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capture| capture["experiment_id"] == "exp-chrome-browser-client")
        .unwrap();
    assert_eq!(
        chrome_provenance["external_producer"]["name"],
        "committed-host-observation-fixture"
    );
    assert_eq!(chrome_provenance["external_producer"]["version"], "1.0.0");
    assert_eq!(
        chrome_provenance["external_producer"]["envelope_digest"],
        sha256_file(&import_root.join("external-capture-envelope.json"))
    );
}

#[test]
fn host_capability_capture_import_rejects_forged_connector_data() {
    let forged_tool = observed_bundle(|root| {
        write_json(
            root,
            "experiment-suite.json",
            &json!({
                "schema_version": "planr.host_capability_experiment_suite.v1",
                "suite_id": "codex-host-capability-phase1",
                "docs_are_experiment_design_only": true,
                "experiments": [{
                    "id": "exp-chrome-browser-client",
                    "host": "codex",
                    "surface": "chrome-browser-client",
                    "input_kind": "controlled_probe",
                    "expected_tool_name": "forged chrome adapter",
                    "expected_event_source": "browser-client:chrome",
                    "expected_result_status": "available"
                }]
            }),
        );
        fs::create_dir_all(root.join("expected")).unwrap();
        fs::create_dir_all(root.join("provenance")).unwrap();
        write_json(
            root,
            "expected/normalized-manifest.json",
            &json!({"forged": true}),
        );
        write_json(
            root,
            "provenance/host-capability-captures.json",
            &json!({"forged": true}),
        );
        let mut envelope = read_json(root, "external-capture-envelope.json");
        let raw = envelope["captures"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|capture| capture["experiment_id"] == "exp-chrome-browser-client")
            .unwrap();
        raw["tool_name"] = json!("forged chrome adapter");
        raw["events"][0]["tool_name"] = json!("forged chrome adapter");
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &forged_tool);
    assert!(
        !output.status.success(),
        "forged import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("tool_name is forged or drifted for exp-chrome-browser-client"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let forged_artifact = observed_bundle(|root| {
        mutate_external_capture(root, |raw| {
            raw["result"]["artifact_refs"][0]["digest"] =
                json!("sha256:1111111111111111111111111111111111111111111111111111111111111111");
        });
    });
    let output = capture_import(tempdir().unwrap().path(), &forged_artifact);
    assert!(
        !output.status.success(),
        "forged artifact import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("digest mismatch"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let reversed_capture_time = observed_bundle(|root| {
        mutate_external_capture(root, |raw| {
            raw["started_at"] = json!("2026-07-29T18:39:17Z");
            raw["ended_at"] = json!("2026-07-29T18:38:17Z");
        });
    });
    let output = capture_import(tempdir().unwrap().path(), &reversed_capture_time);
    assert!(
        !output.status.success(),
        "reversed-time import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("started_at must not be later"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let invalid_calendar = observed_bundle(|root| {
        mutate_external_capture(root, |raw| {
            raw["started_at"] = json!("2026-02-31T00:00:00Z");
            raw["ended_at"] = json!("2026-02-31T00:00:00Z");
        });
    });
    let output = capture_import(tempdir().unwrap().path(), &invalid_calendar);
    assert!(
        !output.status.success(),
        "invalid-calendar import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("valid UTC timestamp"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let producer_before_capture = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        let capture_started_at = OffsetDateTime::parse(
            envelope["captures"][0]["started_at"].as_str().unwrap(),
            &Rfc3339,
        )
        .unwrap();
        envelope["producer"]["captured_at"] = json!(format_utc_second(
            capture_started_at - time::Duration::seconds(1)
        ));
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &producer_before_capture);
    assert!(
        !output.status.success(),
        "producer-before-capture import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("producer.captured_at must not be earlier"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stale_but_coherent = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        envelope["producer"]["captured_at"] = json!("2020-01-01T00:00:01Z");
        envelope["captures"][0]["started_at"] = json!("2020-01-01T00:00:00Z");
        envelope["captures"][0]["ended_at"] = json!("2020-01-01T00:00:00Z");
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &stale_but_coherent);
    assert!(
        !output.status.success(),
        "stale coherent import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("stale for fresh external import"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stale_capture_fresh_producer = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        envelope["producer"]["captured_at"] = json!(current_utc_second());
        envelope["captures"][0]["started_at"] = json!("2020-01-01T00:00:00Z");
        envelope["captures"][0]["ended_at"] = json!("2020-01-01T00:00:00Z");
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &stale_capture_fresh_producer);
    assert!(
        !output.status.success(),
        "stale capture import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("ended_at is stale"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let future_producer = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        envelope["producer"]["captured_at"] = json!("2999-01-01T00:00:00Z");
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &future_producer);
    assert!(
        !output.status.success(),
        "future producer import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("must not be in the future"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let missing_producer_version = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        envelope["producer"]
            .as_object_mut()
            .unwrap()
            .remove("version");
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &missing_producer_version);
    assert!(
        !output.status.success(),
        "missing producer import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("external envelope.producer.version"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let empty_producer_name = observed_bundle(|root| {
        let mut envelope = read_json(root, "external-capture-envelope.json");
        envelope["producer"]["name"] = json!("");
        write_json(root, "external-capture-envelope.json", &envelope);
    });
    let output = capture_import(tempdir().unwrap().path(), &empty_producer_name);
    assert!(
        !output.status.success(),
        "empty producer name import unexpectedly passed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("external envelope.producer.name"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
