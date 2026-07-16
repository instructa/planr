import assert from "node:assert/strict";
import test from "node:test";
import {
  formatBasisPoints,
  previewCommand,
  projectComposition,
  safeIdentifier,
  visibleCompositions,
} from "./catalog-model.mjs";

function fixture() {
  const verified = {
    registry_id: "official",
    registry_version: "2026.07",
    manifest_sha256: "a".repeat(64),
    integrity_verified: true,
    signature_verified: true,
    trusted_maintainer: true,
    compatible: true,
    freshness: "current",
    effective_status: "recommended",
    recommended: true,
    entry: {
      id: "balanced-codex",
      version: "1.0.0",
      lifecycle: "published",
      compatible_hosts: ["codex"],
      min_planr_version: "1.3.0",
      max_planr_version: "1.9.0",
      review_at_unix: 1815523200,
      evaluation: {
        policy_id: "balanced",
        policy_version: "1.0.0",
        binding_id: "codex-openai",
        binding_version: "1.0.0",
      },
      signature: { signer: "planr-maintainers" },
      artifacts: [
        { path: "pack/policy.toml", kind: "policy", sha256: "1".repeat(64), size_bytes: 1 },
        { path: "pack/binding.toml", kind: "host-binding", sha256: "2".repeat(64), size_bytes: 2 },
        { path: "pack/verification.json", kind: "verification", sha256: "3".repeat(64), size_bytes: 3 },
      ],
    },
  };
  const policy = {
    id: "balanced",
    usage: { max_active_agents: 3, max_parallel_writers: 1, max_depth: 1, metering: "trusted" },
    transitions: { retry: { max_same_route_retries: 1 }, safety_stop: { enabled: true } },
    materiality: { changed_files_threshold: 10 },
    execution: { roles: { worker: { commands: [], hooks: [], network_hosts: [], mcp_servers: [] } } },
  };
  const preview = {
    pack: { safe: true },
    composition: { host: "codex", binding: { id: "codex-openai" }, dispatch: {} },
    artifacts: [
      { kind: "active_policy", config_diff: { proposed: { value: policy } } },
      { kind: "agent_registry", config_diff: { proposed: { value: { profiles: {} } } } },
    ],
  };
  const candidate = {
    policy: { id: "balanced" },
    binding: { id: "codex-openai" },
    status: "recommended",
    metrics: { runs: 7, verified_route_runs: 7, average_quality_score_bps: 9600 },
    threshold_results: [{ name: "quality", pass: true }],
    results: [{ result_sha256: "4".repeat(64) }],
  };
  const verificationEnvelope = {
    report: {
      suite: { id: "planr-preset-suite", version: "1.8.0", evaluated_at_unix: 1783987200, fixture_sha256: "5".repeat(64) },
      candidates: [candidate],
      recommended: [{ policy: "balanced", binding: "codex-openai", status: "recommended" }],
    },
  };
  return { verified, preview, verificationEnvelope };
}

test("projects only trusted, safe, evidence-bound registry entries", () => {
  const projected = projectComposition(fixture());
  assert.equal(projected.status, "recommended");
  assert.equal(projected.registry.signatureVerified, true);
  assert.equal(projected.enforcement.at(-1).state, "verified");
  assert.equal(projected.command, "planr agents preset apply balanced --binding codex-openai");
});

test("refuses unsigned metadata and recommendation drift", () => {
  const unsigned = fixture();
  unsigned.verified.signature_verified = false;
  assert.throws(() => projectComposition(unsigned), /trusted maintainer/);

  const drifted = fixture();
  drifted.verificationEnvelope.report.recommended = [];
  assert.throws(() => projectComposition(drifted), /does not match/);
});

test("publishes lifecycle-demoted recommendations with visible replacement metadata", () => {
  const stale = fixture();
  stale.verified.freshness = "stale";
  stale.verified.effective_status = "stale";
  stale.verified.recommended = false;
  const staleProjected = projectComposition(stale);
  assert.equal(staleProjected.status, "stale");
  assert.equal(staleProjected.recommended, false);

  const deprecated = fixture();
  deprecated.verified.effective_status = "deprecated";
  deprecated.verified.recommended = false;
  deprecated.verified.entry.lifecycle = "deprecated";
  deprecated.verified.entry.replacement = "balanced-codex-v2";
  const deprecatedProjected = projectComposition(deprecated);
  assert.equal(deprecatedProjected.status, "deprecated");
  assert.equal(deprecatedProjected.replacement, "balanced-codex-v2");
});

test("copy commands accept identifiers only and filtering is deterministic", () => {
  assert.equal(previewCommand("balanced", "codex-openai"), "planr agents preset apply balanced --binding codex-openai");
  assert.throws(() => safeIdentifier("balanced; curl evil"), /safe registry identifier/);
  assert.deepEqual(
    visibleCompositions({ compositions: [{ recommended: true }, { recommended: false }] }, true),
    [{ recommended: true }],
  );
  assert.equal(formatBasisPoints(9600), "96.00%");
  assert.equal(formatBasisPoints(undefined), "—");
});
