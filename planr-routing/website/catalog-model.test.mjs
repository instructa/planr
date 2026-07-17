import assert from "node:assert/strict";
import test from "node:test";
import {
  formatBasisPoints,
  previewCommand,
  projectComposition,
  safeIdentifier,
  visibleCompositions,
} from "./catalog-model.mjs";

function syntheticUiFixture() {
  const verified = {
    registry_id: "ui-test-registry",
    registry_version: "0.0.0-test",
    manifest_sha256: "a".repeat(64),
    integrity_verified: true,
    signature_verified: true,
    trusted_maintainer: true,
    compatible: true,
    freshness: "current",
    effective_status: "recommended",
    recommended: true,
    entry: {
      id: "ui-test-pack",
      version: "0.0.0-test",
      lifecycle: "published",
      compatible_hosts: ["synthetic-host"],
      min_planr_version: "0.0.0-test",
      max_planr_version: "0.0.0-test",
      review_at_unix: 1815523200,
      evaluation: {
        policy_id: "ui-test-policy",
        policy_version: "0.0.0-test",
        binding_id: "ui-test-binding",
        binding_version: "0.0.0-test",
      },
      signature: { signer: "ui-test-signer" },
      artifacts: [
        { path: "pack/policy.toml", kind: "policy", sha256: "1".repeat(64), size_bytes: 1 },
        { path: "pack/binding.toml", kind: "host-binding", sha256: "2".repeat(64), size_bytes: 2 },
        { path: "pack/verification.json", kind: "verification", sha256: "3".repeat(64), size_bytes: 3 },
      ],
    },
  };
  const policy = {
    id: "ui-test-policy",
    usage: { max_active_agents: 3, max_parallel_writers: 1, max_depth: 1, metering: "trusted" },
    transitions: { retry: { max_same_route_retries: 1 }, safety_stop: { enabled: true } },
    materiality: { changed_files_threshold: 10 },
    execution: { roles: { worker: { commands: [], hooks: [], network_hosts: [], mcp_servers: [] } } },
  };
  const preview = {
    pack: { safe: true },
    composition: { host: "synthetic-host", binding: { id: "ui-test-binding" }, dispatch: {} },
    artifacts: [
      { kind: "active_policy", config_diff: { proposed: { value: policy } } },
      { kind: "agent_registry", config_diff: { proposed: { value: { profiles: {} } } } },
    ],
  };
  const candidate = {
    policy: { id: "ui-test-policy" },
    binding: { id: "ui-test-binding" },
    status: "recommended",
    metrics: { runs: 7, verified_route_runs: 7, average_quality_score_bps: 9600 },
    threshold_results: [{ name: "quality", pass: true }],
    results: [{ result_sha256: "4".repeat(64) }],
  };
  const verificationEnvelope = {
    report: {
      suite: { id: "ui-test-suite", version: "0.0.0-test", evaluated_at_unix: 1783987200, fixture_sha256: "5".repeat(64) },
      candidates: [candidate],
      recommended: [{ policy: "ui-test-policy", binding: "ui-test-binding", status: "recommended" }],
    },
  };
  return { verified, preview, verificationEnvelope };
}

test("projects only trusted, safe, evidence-bound registry entries", () => {
  const projected = projectComposition(syntheticUiFixture());
  assert.equal(projected.status, "recommended");
  assert.equal(projected.registry.signatureVerified, true);
  assert.equal(projected.enforcement.at(-1).state, "verified");
  assert.equal(projected.command, "planr-routing compile ui-test-policy --host ui-test-binding --output routing-bundle.json && planr routing bundle preview routing-bundle.json");
});

test("refuses unsigned metadata and recommendation drift", () => {
  const unsigned = syntheticUiFixture();
  unsigned.verified.signature_verified = false;
  assert.throws(() => projectComposition(unsigned), /trusted maintainer signature/);

  const drifted = syntheticUiFixture();
  drifted.verificationEnvelope.report.recommended = [];
  assert.throws(() => projectComposition(drifted), /does not match/);
});

test("publishes unsigned synthetic candidates only while visibly demoted", () => {
  const experimental = syntheticUiFixture();
  experimental.verified.signature_verified = false;
  experimental.verified.trusted_maintainer = false;
  experimental.verified.effective_status = "experimental";
  experimental.verified.recommended = false;
  experimental.verified.entry.signature = undefined;
  experimental.verificationEnvelope.report.recommended = [];
  experimental.verificationEnvelope.report.candidates[0].status = "verified";

  const projected = projectComposition(experimental);
  assert.equal(projected.status, "experimental");
  assert.equal(projected.recommended, false);
  assert.equal(projected.registry.signatureVerified, false);
  assert.equal(projected.registry.signer, undefined);
});

test("publishes lifecycle-demoted recommendations with visible replacement metadata", () => {
  const stale = syntheticUiFixture();
  stale.verified.freshness = "stale";
  stale.verified.effective_status = "stale";
  stale.verified.recommended = false;
  const staleProjected = projectComposition(stale);
  assert.equal(staleProjected.status, "stale");
  assert.equal(staleProjected.recommended, false);

  const deprecated = syntheticUiFixture();
  deprecated.verified.effective_status = "deprecated";
  deprecated.verified.recommended = false;
  deprecated.verified.entry.lifecycle = "deprecated";
  deprecated.verified.entry.replacement = "ui-test-pack-v2";
  const deprecatedProjected = projectComposition(deprecated);
  assert.equal(deprecatedProjected.status, "deprecated");
  assert.equal(deprecatedProjected.replacement, "ui-test-pack-v2");
});

test("copy commands accept identifiers only and filtering is deterministic", () => {
  assert.equal(previewCommand("ui-test-policy", "ui-test-binding"), "planr-routing compile ui-test-policy --host ui-test-binding --output routing-bundle.json && planr routing bundle preview routing-bundle.json");
  assert.throws(() => safeIdentifier("ui-test; curl invalid"), /safe registry identifier/);
  assert.deepEqual(
    visibleCompositions({ compositions: [{ recommended: true }, { recommended: false }] }, true),
    [{ recommended: true }],
  );
  assert.equal(formatBasisPoints(9600), "96.00%");
  assert.equal(formatBasisPoints(undefined), "—");
});
