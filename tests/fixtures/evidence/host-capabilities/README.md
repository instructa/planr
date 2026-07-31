# Host Capability Experiment Fixtures

Phase 1 records observed host capability contracts before any trusted adapter is enabled.

Set `PLANR_PACKAGE_ROOT` to the source checkout or extracted package root before
running these examples. Each block canonicalizes it with `pwd -P` and uses
absolute harness, fixture, and artifact paths so it works from any caller
directory.

Replay the deterministic fixture suite:

```bash
: "${PLANR_PACKAGE_ROOT:?set PLANR_PACKAGE_ROOT to the source checkout or extracted package root}"
planr_package_root="$(CDPATH=; cd -- "$PLANR_PACKAGE_ROOT" && pwd -P)"
planr_harness="$planr_package_root/scripts/host-capability-experiment.mjs"
planr_fixture_root="$planr_package_root/tests/fixtures/evidence/host-capabilities/v1"
node "$planr_harness" replay --fixture-root "$planr_fixture_root"
```

The replay command resolves the canonical Rust validator as a sibling
entrypoint at `scripts/planr-host-capability-validator`. In a source checkout
that entrypoint uses an existing local build or builds
`planr-host-capability-validator`; in packaged artifacts the native validator is
bundled beside the harness.

Capture mode is for developer research only and writes a new raw corpus for review.
The output directory must be absent or empty, outside the repository workspace,
and must not overlap an import source. The harness generates into a private
staging directory first, then publishes the finished corpus without recursively
deleting caller-controlled paths:

```bash
: "${PLANR_PACKAGE_ROOT:?set PLANR_PACKAGE_ROOT to the source checkout or extracted package root}"
planr_package_root="$(CDPATH=; cd -- "$PLANR_PACKAGE_ROOT" && pwd -P)"
planr_harness="$planr_package_root/scripts/host-capability-experiment.mjs"
planr_capture_tmp="$(mktemp -d)"
planr_capture_tmp="$(CDPATH=; cd -- "$planr_capture_tmp" && pwd -P)"
planr_capture_out="$planr_capture_tmp/planr-host-capabilities"
node "$planr_harness" capture --out-dir "$planr_capture_out"
node "$planr_harness" replay --fixture-root "$planr_capture_out"
rm -rf "$planr_capture_tmp"
```

Connector observations captured outside this process are imported through a
minimal external envelope, not through a whole fixture tree. The import root
contains `external-capture-envelope.json` plus referenced `artifacts/` bytes.
The harness owns the suite, schemas, provenance, manifest, and expected output;
it validates incoming raw captures against package-owned experiment
ID/tool/event/host contracts, copies referenced artifacts, regenerates
provenance and normalized expected output locally, and leaves non-imported
surfaces as explicit unavailable placeholders:

```bash
: "${PLANR_PACKAGE_ROOT:?set PLANR_PACKAGE_ROOT to the source checkout or extracted package root}"
planr_package_root="$(CDPATH=; cd -- "$PLANR_PACKAGE_ROOT" && pwd -P)"
planr_harness="$planr_package_root/scripts/host-capability-experiment.mjs"
planr_capture_tmp="$(mktemp -d)"
planr_capture_tmp="$(CDPATH=; cd -- "$planr_capture_tmp" && pwd -P)"
planr_capture_out="$planr_capture_tmp/planr-host-capabilities"
planr_import_root="$planr_capture_tmp/host-external-envelope"
mkdir -p "$planr_import_root/artifacts/local-function"
PLANR_IMPORT_ROOT="$planr_import_root" node <<'NODE'
const crypto = require("crypto");
const fs = require("fs");
function fixtureLocalFunction(left, right) {
  return { result: "ok", value: left + right };
}
const now = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
const artifactPath = `${process.env.PLANR_IMPORT_ROOT}/artifacts/local-function/result.json`;
fs.writeFileSync(artifactPath, `${JSON.stringify(fixtureLocalFunction(40, 2))}\n`);
const digest = `sha256:${crypto.createHash("sha256").update(fs.readFileSync(artifactPath)).digest("hex")}`;
const raw = {
  schema_version: "planr.host_capability_observed_raw.v1",
  payload_version: "host-capability-raw/1.0.0",
  experiment_id: "exp-local-function",
  host_identity: {
    host: "codex",
    surface: "local-function",
    version: process.version,
    adapter_version: "planr-host-experiment-harness/1.0.0"
  },
  surface: "local-function",
  tool_name: "planr:local-function-fixture",
  event_source: "node:function-call",
  started_at: now,
  ended_at: now,
  input: {
    input_kind: "controlled_probe",
    function: "fixtureLocalFunction",
    args: [40, 2]
  },
  events: [
    {
      sequence: 1,
      event_name: "started",
      payload_version: "host-event/1.0.0",
      tool_name: "planr:local-function-fixture",
      event_source: "node:function-call",
      payload: { input_kind: "controlled_probe" }
    },
    {
      sequence: 2,
      event_name: "final",
      final: true,
      payload_version: "host-event/1.0.0",
      tool_name: "planr:local-function-fixture",
      event_source: "node:function-call",
      payload: { final_status: "available" }
    }
  ],
  result: {
    final_status: "available",
    availability_reason: "controlled local-function import example produced fresh artifact bytes",
    permissions: {
      network: "not_used",
      filesystem: "fixture-controlled",
      environment: "node process environment visible to controlled probe",
      secrets: "not_requested"
    },
    sandbox: { mode: "developer-local", writable_roots: ["fixture_root"] },
    missing_fields: [],
    blind_spots: ["agent transcript and private profile data intentionally not captured"],
    artifact_refs: [{
      id: "artifact-local-function-result",
      kind: "json-result",
      root_kind: "fixture_root",
      path: "artifacts/local-function/result.json",
      digest
    }],
    artifact_digests: { "artifact-local-function-result": digest }
  }
};
fs.writeFileSync(`${process.env.PLANR_IMPORT_ROOT}/external-capture-envelope.json`, `${JSON.stringify({
  schema_version: "planr.host_capability_external_capture_envelope.v1",
  producer: {
    name: "local-function-import-example",
    version: "1.0.0",
    captured_at: now
  },
  suite_id: "codex-host-capability-phase1",
  captures: [raw]
}, null, 2)}\n`);
NODE
node "$planr_harness" capture --out-dir "$planr_capture_out" --import-fixture-root "$planr_import_root"
node "$planr_harness" replay --fixture-root "$planr_capture_out"
PLANR_CAPTURE_OUT="$planr_capture_out" PLANR_IMPORT_ROOT="$planr_import_root" node <<'NODE'
const crypto = require("crypto");
const fs = require("fs");
const out = process.env.PLANR_CAPTURE_OUT;
const importedArtifact = `${out}/artifacts/local-function/result.json`;
const artifactBytes = fs.readFileSync(importedArtifact);
if (artifactBytes.toString("utf8") !== '{"result":"ok","value":42}\n') {
  throw new Error("imported local-function artifact bytes drifted");
}
const digest = `sha256:${crypto.createHash("sha256").update(artifactBytes).digest("hex")}`;
const raw = JSON.parse(fs.readFileSync(`${out}/observed/exp-local-function.json`, "utf8"));
if (raw.result.artifact_refs[0].digest !== digest || raw.result.artifact_digests["artifact-local-function-result"] !== digest) {
  throw new Error("imported local-function artifact digest drifted");
}
const provenance = JSON.parse(fs.readFileSync(`${out}/provenance/host-capability-captures.json`, "utf8"));
const capture = provenance.captures.find((entry) => entry.experiment_id === "exp-local-function");
const envelopeBytes = fs.readFileSync(`${process.env.PLANR_IMPORT_ROOT}/external-capture-envelope.json`);
const envelopeDigest = `sha256:${crypto.createHash("sha256").update(envelopeBytes).digest("hex")}`;
if (!capture || capture.external_producer?.name !== "local-function-import-example" || capture.external_producer?.envelope_digest !== envelopeDigest) {
  throw new Error("imported local-function producer provenance drifted");
}
NODE
rm -rf "$planr_capture_tmp"
```

Documentation links in the suite define experiment targets only. They are never proof of support.
