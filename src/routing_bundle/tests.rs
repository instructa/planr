use super::*;
use ed25519_dalek::{Signer, SigningKey};
use tempfile::tempdir;

const VALID: &str =
    include_str!("../../planr-routing/fixtures/routing-bundle-v1/valid-balanced-codex.json");
const UNSUPPORTED_VERSION: &str =
    include_str!("../../planr-routing/fixtures/routing-bundle-v1/invalid-unsupported-version.json");
const BAD_ARTIFACT_HASH: &str =
    include_str!("../../planr-routing/fixtures/routing-bundle-v1/invalid-artifact-hash.json");
const DUAL_PAYLOAD: &str = include_str!(
    "../../planr-routing/fixtures/routing-bundle-v1/invalid-dual-artifact-payload.json"
);

#[test]
fn package_owned_valid_fixture_is_consumed_by_core() {
    let bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    bundle.validate().unwrap();
    assert_eq!(bundle.schema_version, ROUTING_BUNDLE_SCHEMA_VERSION);
    assert_eq!(bundle.source.package, "planr-routing");
    assert_eq!(bundle.policy_id, "balanced");
    assert!(!bundle.profiles.is_empty());
    assert!(!bundle.routes.is_empty());
    assert!(!bundle.artifacts.is_empty());
    assert!(matches!(
        bundle.artifacts[0].payload,
        RoutingArtifactPayloadV1::Inline(_)
    ));
}

#[test]
fn unsupported_schema_version_is_rejected_without_coercion() {
    let bundle = RoutingBundleV1::parse_json(UNSUPPORTED_VERSION).unwrap();
    assert_eq!(
        bundle.validate(),
        Err(RoutingBundleError::UnsupportedSchemaVersion { found: 2 })
    );
}

#[test]
fn artifact_content_must_match_declared_digest() {
    let bundle = RoutingBundleV1::parse_json(BAD_ARTIFACT_HASH).unwrap();
    assert!(matches!(
        bundle.validate(),
        Err(RoutingBundleError::ArtifactDigestMismatch { .. })
    ));
}

#[test]
fn artifact_payload_has_one_canonical_shape() {
    let error = RoutingBundleV1::parse_json(DUAL_PAYLOAD).unwrap_err();
    assert!(error.to_string().contains("bundle"));
}

#[test]
fn unsigned_bundle_cannot_self_assert_verified_evidence() {
    let mut bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    bundle.evidence.as_mut().unwrap().status = RoutingEvidenceStatusV1::Recommended;
    assert!(
        bundle
            .validate()
            .unwrap_err()
            .to_string()
            .contains("externally trusted bundle signature")
    );
}

#[test]
fn application_record_contract_validates_current_shape() {
    let record = RoutingApplicationRecordV1 {
        schema_version: ROUTING_APPLICATION_RECORD_SCHEMA_VERSION,
        bundle_id: "balanced-codex@1.0.0".into(),
        bundle_sha256: "0".repeat(64),
        repository_id: "sha256:repository-fixture".into(),
        previewed_at: "2026-07-16T00:00:00Z".into(),
        applied_at: None,
        artifacts: vec![RoutingArtifactReceiptV1 {
            path: ".codex/agents/planr-terra-high.toml".into(),
            proposed_sha256: "1".repeat(64),
            previous_sha256: None,
            outcome: RoutingArtifactOutcomeV1::Planned,
        }],
        conflicts: vec![],
        declared_routes: vec![RoutingDeclaredRouteEvidenceV1 {
            selector: "work_type=code".into(),
            profile: "implementer".into(),
        }],
        effective_routes: vec![],
    };
    record.validate().unwrap();
}

#[test]
fn preview_apply_and_second_apply_are_deterministic() {
    let root = tempdir().unwrap();
    let bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    let preview = preview_bundle(root.path(), &bundle, None).unwrap();
    assert!(preview.is_applicable());
    assert_eq!(preview.artifacts[0].action, RoutingArtifactActionV1::Create);

    let (_, receipts) = apply_bundle(root.path(), &bundle, None).unwrap();
    assert_eq!(receipts[0].outcome, RoutingArtifactOutcomeV1::Created);
    let created = root.path().join(&bundle.artifacts[0].path);
    let RoutingArtifactPayloadV1::Inline(expected) = &bundle.artifacts[0].payload else {
        panic!("fixture artifact must be inline");
    };
    assert_eq!(std::fs::read(created).unwrap(), expected.content.as_bytes());

    let second = preview_bundle(root.path(), &bundle, None).unwrap();
    assert_eq!(
        second.artifacts[0].action,
        RoutingArtifactActionV1::Unchanged
    );
    let (_, receipts) = apply_bundle(root.path(), &bundle, None).unwrap();
    assert_eq!(receipts[0].outcome, RoutingArtifactOutcomeV1::Unchanged);
}

#[test]
fn replace_mode_is_explicit_and_hash_evidenced() {
    let root = tempdir().unwrap();
    let mut bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    let target = root.path().join(&bundle.artifacts[0].path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "old").unwrap();
    bundle.artifacts[0].mode = RoutingArtifactModeV1::Replace;
    bundle.artifacts[0].payload = RoutingArtifactPayloadV1::Inline(RoutingInlineContentV1 {
        content: "new".into(),
    });
    bundle.artifacts[0].sha256 = sha256(b"new");

    let preview = preview_bundle(root.path(), &bundle, None).unwrap();
    assert_eq!(
        preview.artifacts[0].action,
        RoutingArtifactActionV1::Replace
    );
    let (_, receipts) = apply_bundle(root.path(), &bundle, None).unwrap();
    assert_eq!(receipts[0].outcome, RoutingArtifactOutcomeV1::Replaced);
    let old_sha256 = sha256(b"old");
    assert_eq!(
        receipts[0].previous_sha256.as_deref(),
        Some(old_sha256.as_str())
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "new");
}

#[test]
fn conflicts_and_unsafe_paths_fail_before_any_write() {
    let root = tempdir().unwrap();
    let bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    let target = root.path().join(&bundle.artifacts[0].path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "unrelated").unwrap();
    let preview = preview_bundle(root.path(), &bundle, None).unwrap();
    assert!(!preview.is_applicable());
    assert!(apply_bundle(root.path(), &bundle, None).is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "unrelated");

    for path in [
        "../escape",
        "/tmp/escape",
        "~/.codex/config.toml",
        ".codex/config.toml",
        "outside/generated.txt",
    ] {
        let mut unsafe_bundle = bundle.clone();
        unsafe_bundle.artifacts[0].path = path.to_string();
        assert!(
            preview_bundle(root.path(), &unsafe_bundle, None).is_err(),
            "{path}"
        );
    }
}

#[test]
fn parent_child_artifact_collisions_fail_before_creating_directories() {
    let root = tempdir().unwrap();
    let mut bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    let mut parent = bundle.artifacts[0].clone();
    parent.path = ".planr/generated".into();
    let mut child = bundle.artifacts[0].clone();
    child.path = ".planr/generated/role.toml".into();
    bundle.artifacts = vec![parent, child];

    assert!(matches!(
        bundle.validate(),
        Err(RoutingBundleError::ArtifactPathCollision { .. })
    ));
    assert!(preview_bundle(root.path(), &bundle, None).is_err());
    assert!(!root.path().join(".planr").exists());
}

#[test]
fn injected_directory_creation_failure_is_journaled_and_rolled_back() {
    let root = tempdir().unwrap();
    let parent = root.path().join(".codex/agents/nested");
    let mut created = Vec::new();
    let mut calls = 0;
    let error = create_missing_directories_with(root.path(), &parent, &mut created, |directory| {
        calls += 1;
        if calls == 2 {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected create failure",
            ))
        } else {
            std::fs::create_dir(directory)
        }
    })
    .unwrap_err();
    assert!(error.to_string().contains("injected create failure"));
    assert_eq!(created.len(), 1);
    assert!(created[0].exists());

    rollback_transaction(&[], &[], &created).unwrap();
    assert!(!root.path().join(".codex").exists());
}

#[test]
fn injected_backup_cleanup_failure_restores_previous_content() {
    let root = tempdir().unwrap();
    let target = root.path().join("target.toml");
    let backup = root.path().join("target.backup");
    std::fs::write(&target, "new").unwrap();
    std::fs::write(&backup, "old").unwrap();
    let committed = vec![CommittedArtifact {
        target: target.clone(),
        backup: Some(backup.clone()),
        previous_content: Some(b"old".to_vec()),
    }];

    let error = cleanup_backups_with(&committed, |_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected backup cleanup failure",
        ))
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("injected backup cleanup failure")
    );

    rollback_transaction(&[], &committed, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    assert!(!backup.exists());
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected_before_writes() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".codex")).unwrap();
    symlink(outside.path(), root.path().join(".codex/agents")).unwrap();
    let bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    let error = preview_bundle(root.path(), &bundle, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("symlink"));
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn signatures_cover_the_unsigned_canonical_bundle() {
    let mut bundle = RoutingBundleV1::parse_json(VALID).unwrap();
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signature = signing_key.sign(&bundle.signing_bytes().unwrap());
    bundle.signature = Some(RoutingBundleSignatureV1 {
        algorithm: RoutingSignatureAlgorithmV1::Ed25519,
        signer: "test-fixture".into(),
        value: encode_hex(&signature.to_bytes()),
    });
    assert_eq!(
        bundle.validate(),
        Err(RoutingBundleError::SignatureTrustRequired)
    );
    let trust = RoutingBundleTrustAnchorV1 {
        signer: "test-fixture".into(),
        public_key: encode_hex(signing_key.verifying_key().as_bytes()),
    };
    bundle.validate_with_trust(Some(&trust)).unwrap();
    let attacker = RoutingBundleTrustAnchorV1 {
        signer: "test-fixture".into(),
        public_key: encode_hex(
            SigningKey::from_bytes(&[8_u8; 32])
                .verifying_key()
                .as_bytes(),
        ),
    };
    assert_eq!(
        bundle.validate_with_trust(Some(&attacker)),
        Err(RoutingBundleError::InvalidSignature)
    );
    let root = tempdir().unwrap();
    assert!(
        preview_bundle(root.path(), &bundle, None)
            .unwrap_err()
            .to_string()
            .contains("external trusted signer")
    );
    preview_bundle(root.path(), &bundle, Some(&trust)).unwrap();

    bundle.policy_version = "tampered".into();
    assert_eq!(
        bundle.validate_with_trust(Some(&trust)),
        Err(RoutingBundleError::InvalidSignature)
    );
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
