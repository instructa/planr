#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  rmdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const RAW_SCHEMA = "planr.host_capability_observed_raw.v1";
const EXPECTED_SCHEMA = "planr.host_capability_expected_manifest.v1";
const SUITE_SCHEMA = "planr.host_capability_experiment_suite.v1";
const RAW_PAYLOAD_PREFIX = "host-capability-raw/";
const EXPECTED_PAYLOAD_PREFIX = "host-capability-expected/";
const SUMMARY_SCHEMA = "planr.host_capability_experiment_summary.v1";
const EXTERNAL_CAPTURE_ENVELOPE_SCHEMA = "planr.host_capability_external_capture_envelope.v1";
const EXTERNAL_CAPTURE_ENVELOPE_PATH = "external-capture-envelope.json";
const ADAPTER_VERSION = "planr-host-experiment-harness/1.0.0";
const MANIFEST_REF_PATH = "manifests/phase1-host-capability-manifests.json";
const RAW_SCHEMA_REF_PATH = "schemas/host-capability-observed-raw.schema.json";
const EXPECTED_SCHEMA_REF_PATH = "schemas/host-capability-expected-manifest.schema.json";
const PROVENANCE_SCHEMA_REF_PATH = "schemas/host-capability-provenance.schema.json";
const PROVENANCE_REF_PATH = "provenance/host-capability-captures.json";
const VALIDATOR_IDENTITY_SCHEMA = "planr.host_capability_validator_identity.v1";
const VALIDATOR_RESULT_SCHEMA = "planr.host_capability_validator_result.v1";
const VALIDATOR_NAME = "planr-host-capability-validator";
const VALIDATOR_VERSION = "1.0.0";
const VALIDATOR_IDENTITY_FIELDS = new Set(["schema_version", "validator", "validator_version"]);
const VALIDATOR_RESULT_FIELDS = new Set([
  "schema_version",
  "validator",
  "validator_version",
  "verdict",
  "input_digest",
  "validated_raw_documents",
  "validated_instances",
]);

const EVIDENCE_AVAILABILITY_STATUSES = new Set([
  "available",
  "unavailable",
  "degraded",
  "permission_denied",
  "sandbox_blocked",
  "unsupported",
  "probe_failed",
]);
const EVIDENCE_ATTEMPT_STATUSES = new Set([
  "passed",
  "failed",
  "skipped",
  "timed_out",
  "aborted",
  "unavailable",
  "inconclusive",
]);
const OBSERVED_CLAIM_SOURCES = new Set(["observed_capture"]);
const CLAIM_SOURCE_RULES = {
  observed_capture: {
    sourceKind: "external_observed_capture",
    observationMode: "observed_payload",
    inputKinds: new Set(["controlled_probe"]),
  },
  mechanical_unavailable_probe: {
    sourceKind: "mechanical_unavailable_probe",
    observationMode: "mechanical_invocation",
    inputKinds: new Set(["mechanical_availability_probe"]),
  },
  capture_mode_placeholder: {
    sourceKind: "unprobed_placeholder",
    observationMode: "unprobed_placeholder",
    inputKinds: new Set(["unprobed_placeholder"]),
  },
};
const DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/;
const EXPERIMENT_ID_PATTERN = /^exp-[a-z0-9]+(?:-[a-z0-9]+)*$/;
const EXTERNAL_CAPTURE_MAX_AGE_MS = 24 * 60 * 60 * 1000;
const EXTERNAL_CAPTURE_FUTURE_SKEW_MS = 5 * 60 * 1000;
const IMPORT_METADATA = Symbol("planrHostExternalImportMetadata");

const RAW_TOP_LEVEL_FIELDS = new Set([
  "schema_version",
  "payload_version",
  "experiment_id",
  "host_identity",
  "surface",
  "tool_name",
  "event_source",
  "started_at",
  "ended_at",
  "input",
  "events",
  "result",
  "provenance_ref",
]);
const RAW_INPUT_FIELDS = new Set([
  "input_kind",
  "command",
  "cwd",
  "javascript",
  "probe",
  "replay_mode",
  "attempts",
  "reset_between_attempts",
  "tool",
  "arguments",
  "navigation",
  "setup",
  "operations",
  "ui_write_actions",
  "function",
  "args",
]);
const RAW_RESULT_FIELDS = new Set([
  "final_status",
  "permissions",
  "sandbox",
  "availability_reason",
  "missing_fields",
  "blind_spots",
  "artifact_refs",
  "artifact_digests",
  "experiment_plan",
  "notes",
]);
const CLAIM_SOURCES = new Set([
  "observed_capture",
  "mechanical_unavailable_probe",
  "capture_mode_placeholder",
]);
const DIGEST_SUBSTRING_PATTERN = /sha256:[0-9a-f]{64}/;

function usage() {
  console.error(
    "Usage: node scripts/host-capability-experiment.mjs replay --fixture-root <dir>\n" +
      "       node scripts/host-capability-experiment.mjs capture --out-dir <dir> [--import-fixture-root <external-envelope-dir>]",
  );
}

function fail(message) {
  throw new Error(message);
}

function packageRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
}

function packagedFixtureRoot() {
  return path.join(packageRoot(), "tests/fixtures/evidence/host-capabilities");
}

function packagedRuntimeRoot() {
  return path.join(packageRoot(), "scripts/host-capability-runtime");
}

function readJson(file) {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    fail(`${file} must be readable JSON: ${error.message}`);
  }
}

function writeJson(file, value) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function sha256Prefixed(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function sha256File(file) {
  return sha256Prefixed(readFileSync(file));
}

function stableDigest(value) {
  return sha256Prefixed(JSON.stringify(value));
}

function assertObject(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  return value;
}

function assertOnlyKeys(value, allowed, label) {
  assertObject(value, label);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      fail(`${label}.${key} is not part of the v1 contract`);
    }
  }
}

function assertArray(value, label) {
  if (!Array.isArray(value)) {
    fail(`${label} must be an array`);
  }
  return value;
}

function assertNonEmptyString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${label} must be a non-empty string`);
  }
  return value;
}

function assertTimestamp(value, label) {
  assertNonEmptyString(value, label);
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(value)) {
    fail(`${label} must be a deterministic UTC timestamp`);
  }
  parseUtcTimestamp(value, label);
}

function parseUtcTimestamp(value, label) {
  assertNonEmptyString(value, label);
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(value)) {
    fail(`${label} must be a deterministic UTC timestamp`);
  }
  const millis = Date.parse(value);
  if (!Number.isFinite(millis)) {
    fail(`${label} must be a valid UTC timestamp`);
  }
  const canonical = new Date(millis).toISOString().replace(/\.\d{3}Z$/, "Z");
  if (canonical !== value) {
    fail(`${label} must be a valid UTC timestamp`);
  }
  return millis;
}

function assertTimeRange(startedAt, endedAt, label) {
  const started = parseUtcTimestamp(startedAt, `${label}.started_at`);
  const ended = parseUtcTimestamp(endedAt, `${label}.ended_at`);
  if (started > ended) {
    fail(`${label}.started_at must not be later than ended_at`);
  }
  return { started, ended };
}

function assertExperimentId(value, label) {
  const id = assertNonEmptyString(value, label);
  if (!EXPERIMENT_ID_PATTERN.test(id)) {
    fail(`${label} must be a canonical experiment id`);
  }
  return id;
}

function assertDigest(value, label) {
  assertNonEmptyString(value, label);
  if (!DIGEST_PATTERN.test(value)) {
    fail(`${label} must be sha256:<64 lowercase hex>`);
  }
}

function assertMajorOne(schema, expected, label) {
  if (schema !== expected) {
    const version = assertNonEmptyString(schema, label).match(/\.v(\d+)$/);
    if (version && Number(version[1]) !== 1) {
      fail(`${label} has unsupported major version ${version[1]}`);
    }
    fail(`${label} must be ${expected}`);
  }
}

function assertPayloadMajorOne(payloadVersion, prefix, label) {
  const value = assertNonEmptyString(payloadVersion, label);
  if (!value.startsWith(prefix)) {
    fail(`${label} must start with ${prefix}`);
  }
  const major = Number(value.slice(prefix.length).split(".")[0]);
  if (!Number.isInteger(major) || major !== 1) {
    fail(`${label} has unsupported major version ${major}`);
  }
}

function safeFixturePath(root, relativePath, label) {
  const value = assertNonEmptyString(relativePath, label);
  if (path.isAbsolute(value) || value.split(/[\\/]/).includes("..")) {
    fail(`${label} must be a relative fixture path without parent traversal`);
  }
  const resolved = path.resolve(root, value);
  const resolvedRoot = path.resolve(root);
  if (!resolved.startsWith(`${resolvedRoot}${path.sep}`) && resolved !== resolvedRoot) {
    fail(`${label} escapes fixture root`);
  }
  return resolved;
}

function readdirSyncSorted(dir) {
  try {
    return readdirSync(dir).sort();
  } catch (error) {
    fail(`${dir} must be readable: ${error.message}`);
  }
}

function loadSuite(fixtureRoot) {
  const suitePath = path.join(fixtureRoot, "experiment-suite.json");
  const suite = assertObject(readJson(suitePath), suitePath);
  assertOnlyKeys(
    suite,
    new Set(["schema_version", "suite_id", "docs_are_experiment_design_only", "source_docs", "experiments"]),
    "suite",
  );
  assertMajorOne(suite.schema_version, SUITE_SCHEMA, "suite.schema_version");
  assertNonEmptyString(suite.suite_id, "suite.suite_id");
  if (suite.source_docs !== undefined) {
    assertArray(suite.source_docs, "suite.source_docs");
  }
  const experiments = new Map();
  for (const experiment of assertArray(suite.experiments, "suite.experiments")) {
    assertObject(experiment, "suite.experiments[]");
    assertOnlyKeys(
      experiment,
      new Set([
        "id",
        "host",
        "surface",
        "input_kind",
        "expected_tool_name",
        "expected_event_source",
        "expected_result_status",
      ]),
      "suite.experiments[]",
    );
    const id = assertExperimentId(experiment.id, "experiment.id");
    if (experiments.has(id)) {
      fail(`duplicate experiment id ${id}`);
    }
    for (const field of [
      "surface",
      "host",
      "expected_tool_name",
      "expected_event_source",
      "expected_result_status",
      "input_kind",
    ]) {
      assertNonEmptyString(experiment[field], `experiment.${field}`);
    }
    if (!EVIDENCE_AVAILABILITY_STATUSES.has(experiment.expected_result_status)) {
      fail(`experiment ${id} uses unknown Evidence availability status`);
    }
    experiments.set(id, experiment);
  }
  if (experiments.size === 0) {
    fail("suite.experiments must not be empty");
  }
  return { suite, experiments };
}

function loadSchemaRefs(fixtureRoot, expected) {
  const schemaRefs = assertObject(expected.schema_refs, "expected.schema_refs");
  assertOnlyKeys(schemaRefs, new Set(["raw", "expected", "provenance"]), "expected.schema_refs");
  return {
    raw: validateFileRef(fixtureRoot, schemaRefs.raw, "expected.schema_refs.raw"),
    expected: validateFileRef(fixtureRoot, schemaRefs.expected, "expected.schema_refs.expected"),
    provenance: validateFileRef(fixtureRoot, schemaRefs.provenance, "expected.schema_refs.provenance"),
  };
}

function validateFileRef(fixtureRoot, ref, label) {
  assertOnlyKeys(ref, new Set(["path", "digest"]), label);
  const file = safeFixturePath(fixtureRoot, ref.path, `${label}.path`);
  if (!existsSync(file)) {
    fail(`${label}.path points at a missing file`);
  }
  assertDigest(ref.digest, `${label}.digest`);
  const digest = sha256File(file);
  if (ref.digest !== digest) {
    fail(`${label}.digest mismatch`);
  }
  return { path: ref.path, digest, document: readJson(file) };
}

function loadProvenance(fixtureRoot, provenanceRef, schemaRefs) {
  const ref = validateFileRef(fixtureRoot, provenanceRef, "provenance_ref");
  const record = assertObject(readJson(safeFixturePath(fixtureRoot, ref.path, "provenance_ref.path")), ref.path);
  assertOnlyKeys(record, new Set(["schema_version", "schema_ref", "schema_digest", "suite_id", "captures"]), "provenance");
  assertMajorOne(record.schema_version, "planr.host_capability_provenance.v1", "provenance.schema_version");
  if (record.schema_ref !== schemaRefs.provenance.path || record.schema_digest !== schemaRefs.provenance.digest) {
    fail("provenance schema_ref/schema_digest must match expected schema_refs.provenance");
  }
  const byId = new Map();
  for (const capture of assertArray(record.captures, "provenance.captures")) {
    assertOnlyKeys(
      capture,
      new Set([
        "experiment_id",
        "source_kind",
        "host",
        "surface",
        "input_kind",
        "observation_mode",
        "tool_name",
        "event_source",
        "host_version",
        "adapter_version",
        "claim_source",
        "availability_reason",
        "probe_checks",
        "missing_fields",
        "artifact_ids",
        "captured_at",
        "external_producer",
      ]),
      "provenance.captures[]",
    );
    const id = assertNonEmptyString(capture.experiment_id, "provenance.captures[].experiment_id");
    if (byId.has(id)) {
      fail(`duplicate provenance capture ${id}`);
    }
    if (!CLAIM_SOURCES.has(assertNonEmptyString(capture.claim_source, "provenance.captures[].claim_source"))) {
      fail(`provenance ${id} has unsupported claim_source`);
    }
    assertNonEmptyString(capture.observation_mode, `provenance ${id}.observation_mode`);
    validateProbeChecks(capture.probe_checks, `provenance ${id}.probe_checks`);
    validateOptionalStringList(assertArray(capture.missing_fields, `provenance ${id}.missing_fields`), `provenance ${id}.missing_fields`);
    validateOptionalStringList(assertArray(capture.artifact_ids, `provenance ${id}.artifact_ids`), `provenance ${id}.artifact_ids`);
    assertTimestamp(capture.captured_at, `provenance ${id}.captured_at`);
    if (capture.external_producer !== undefined) {
      assertOnlyKeys(
        capture.external_producer,
        new Set(["name", "version", "captured_at", "envelope_digest"]),
        `provenance ${id}.external_producer`,
      );
      assertNonEmptyString(capture.external_producer.name, `provenance ${id}.external_producer.name`);
      assertNonEmptyString(capture.external_producer.version, `provenance ${id}.external_producer.version`);
      assertTimestamp(capture.external_producer.captured_at, `provenance ${id}.external_producer.captured_at`);
      assertDigest(capture.external_producer.envelope_digest, `provenance ${id}.external_producer.envelope_digest`);
      if (parseUtcTimestamp(capture.external_producer.captured_at, `provenance ${id}.external_producer.captured_at`) < parseUtcTimestamp(capture.captured_at, `provenance ${id}.captured_at`)) {
        fail(`provenance ${id}.external_producer.captured_at must not be earlier than captured_at`);
      }
    }
    byId.set(id, capture);
  }
  return { ref, record, byId };
}

function loadRawCaptures(fixtureRoot, experiments) {
  const observedDir = path.join(fixtureRoot, "observed");
  const rawById = new Map();
  for (const fileName of readdirSyncSorted(observedDir)) {
    if (!fileName.endsWith(".json")) {
      continue;
    }
    const file = path.join(observedDir, fileName);
    const raw = assertObject(readJson(file), file);
    validateRawCapture(fixtureRoot, raw, experiments, file);
    if (rawById.has(raw.experiment_id)) {
      fail(`duplicate raw capture for ${raw.experiment_id}`);
    }
    rawById.set(raw.experiment_id, raw);
  }
  for (const id of experiments.keys()) {
    if (!rawById.has(id)) {
      fail(`missing raw capture for experiment ${id}`);
    }
  }
  validateArtifactGraph(fixtureRoot, rawById);
  return rawById;
}

function validateRawCapture(fixtureRoot, raw, experiments, file) {
  assertOnlyKeys(raw, RAW_TOP_LEVEL_FIELDS, file);
  assertMajorOne(raw.schema_version, RAW_SCHEMA, `${file}.schema_version`);
  assertPayloadMajorOne(raw.payload_version, RAW_PAYLOAD_PREFIX, `${file}.payload_version`);
  const experimentId = assertExperimentId(raw.experiment_id, `${file}.experiment_id`);
  const experiment = experiments.get(experimentId);
  if (!experiment) {
    fail(`${file} references unknown experiment ${experimentId}`);
  }
  assertOnlyKeys(raw.host_identity, new Set(["host", "surface", "version", "adapter_version"]), `${file}.host_identity`);
  for (const field of ["host", "surface", "version", "adapter_version"]) {
    assertNonEmptyString(raw.host_identity[field], `${file}.host_identity.${field}`);
  }
  if (raw.host_identity.host !== experiment.host) {
    fail(`${file}.host_identity.host does not match experiment ${experimentId}`);
  }
  if (raw.host_identity.surface !== experiment.surface || raw.surface !== experiment.surface) {
    fail(`${file}.host_identity.surface does not match experiment ${experimentId}`);
  }
  if (raw.tool_name !== experiment.expected_tool_name) {
    fail(`${file}.tool_name is forged or drifted for ${experimentId}`);
  }
  if (raw.event_source !== experiment.expected_event_source) {
    fail(`${file}.event_source is forged or drifted for ${experimentId}`);
  }
  assertOnlyKeys(raw.input, RAW_INPUT_FIELDS, `${file}.input`);
  if (raw.input.input_kind !== experiment.input_kind) {
    fail(`${file}.input.input_kind does not match experiment ${experimentId}`);
  }
  assertOnlyKeys(raw.result, RAW_RESULT_FIELDS, `${file}.result`);
  assertTimeRange(raw.started_at, raw.ended_at, file);
  validateRawEvents(raw, experiment, file);
  validateRawResult(fixtureRoot, raw, experiment, file);
  assertNoUnboundDigests(raw, file, (segments) => {
    return (
      isPath(segments, ["result", "artifact_refs", "*", "digest"]) ||
      isPath(segments, ["provenance_ref", "digest"]) ||
      (segments.length === 3 && segments[0] === "result" && segments[1] === "artifact_digests")
    );
  });
}

function validateRawEvents(raw, experiment, file) {
  const events = assertArray(raw.events, `${file}.events`);
  const finalEvents = events.filter((event) => event && event.final === true);
  if (finalEvents.length !== 1) {
    fail(`${file} must contain exactly one final event`);
  }
  const seenSequences = new Set();
  for (const [index, event] of events.entries()) {
    assertObject(event, `${file}.events[${index}]`);
    assertOnlyKeys(
      event,
      new Set(["sequence", "event_name", "final", "payload_version", "tool_name", "event_source", "payload"]),
      `${file}.events[${index}]`,
    );
    if (!Number.isInteger(event.sequence) || event.sequence < 1) {
      fail(`${file}.events[${index}].sequence must be a positive integer`);
    }
    if (seenSequences.has(event.sequence)) {
      fail(`${file}.events[${index}].sequence is duplicated`);
    }
    seenSequences.add(event.sequence);
    assertNonEmptyString(event.event_name, `${file}.events[${index}].event_name`);
    assertPayloadMajorOne(event.payload_version, "host-event/", `${file}.events[${index}].payload_version`);
    assertOnlyKeys(
      event.payload,
      new Set([
        "input_kind",
        "final_status",
        "exit_code",
        "title",
        "url",
        "bodyVisible",
        "heading_count",
        "isError",
        "redacted_message",
  "call_tool_keys",
        "status",
      ]),
      `${file}.events[${index}].payload`,
    );
    if (event.payload.input_kind && event.payload.input_kind !== experiment.input_kind) {
      fail(`${file}.events[${index}].payload.input_kind does not match experiment ${raw.experiment_id}`);
    }
    if (event.tool_name && event.tool_name !== experiment.expected_tool_name) {
      fail(`${file}.events[${index}].tool_name is forged or drifted`);
    }
    if (event.event_source && event.event_source !== experiment.expected_event_source) {
      fail(`${file}.events[${index}].event_source is forged or drifted`);
    }
  }
  const finalEvent = finalEvents[0];
  if (events[events.length - 1] !== finalEvent) {
    fail(`${file} final event must be the last event`);
  }
  if (finalEvent.sequence !== Math.max(...events.map((event) => event.sequence))) {
    fail(`${file} final event must have the last sequence`);
  }
  if (finalEvent.event_name !== "final") {
    fail(`${file} final event must be named final`);
  }
  if (finalEvent.payload.final_status !== raw.result.final_status) {
    fail(`${file} final event status must match result.final_status`);
  }
}

function validateRawResult(fixtureRoot, raw, experiment, file) {
  const finalStatus = assertNonEmptyString(raw.result.final_status, `${file}.result.final_status`);
  if (!EVIDENCE_AVAILABILITY_STATUSES.has(finalStatus)) {
    fail(`${file}.result.final_status must reuse Evidence availability status vocabulary`);
  }
  if (finalStatus !== experiment.expected_result_status) {
    fail(`${file}.result.final_status does not match the experiment contract`);
  }
  validatePermissionState(raw.result.permissions, `${file}.result.permissions`);
  assertOnlyKeys(raw.result.sandbox, new Set(["mode", "writable_roots"]), `${file}.result.sandbox`);
  assertNonEmptyString(raw.result.sandbox.mode, `${file}.result.sandbox.mode`);
  validateOptionalStringList(assertArray(raw.result.sandbox.writable_roots, `${file}.result.sandbox.writable_roots`), `${file}.result.sandbox.writable_roots`);
  validateOptionalStringList(assertArray(raw.result.missing_fields, `${file}.result.missing_fields`), `${file}.result.missing_fields`);
  validateOptionalStringList(assertArray(raw.result.blind_spots, `${file}.result.blind_spots`), `${file}.result.blind_spots`);
  if (raw.result.experiment_plan !== undefined) {
    validateOptionalStringList(assertArray(raw.result.experiment_plan, `${file}.result.experiment_plan`), `${file}.result.experiment_plan`);
  }
  if (raw.result.notes !== undefined) {
    validateOptionalStringList(assertArray(raw.result.notes, `${file}.result.notes`), `${file}.result.notes`);
  }
  const artifactRefs = assertArray(raw.result.artifact_refs, `${file}.result.artifact_refs`);
  assertObject(raw.result.artifact_digests, `${file}.result.artifact_digests`);
  const artifactIds = new Set(artifactRefs.map((artifactRef) => artifactRef.id));
  for (const artifactId of Object.keys(raw.result.artifact_digests)) {
    if (!artifactIds.has(artifactId)) {
      fail(`${file}.result.artifact_digests.${artifactId} has no matching artifact_ref`);
    }
  }
  for (const artifactRef of artifactRefs) {
    validateArtifactRef(fixtureRoot, artifactRef, raw.result.artifact_digests, file);
  }
}

function validateArtifactRef(fixtureRoot, artifactRef, artifactDigests, file) {
  assertOnlyKeys(artifactRef, new Set(["id", "kind", "root_kind", "path", "digest"]), `${file}.result.artifact_refs[]`);
  const id = assertNonEmptyString(artifactRef.id, `${file}.artifact.id`);
  assertNonEmptyString(artifactRef.kind, `${file}.artifact.kind`);
  if (artifactRef.root_kind !== "fixture_root") {
    fail(`${file}.artifact ${id} root_kind must be fixture_root`);
  }
  const artifactPath = safeFixturePath(fixtureRoot, artifactRef.path, `${file}.artifact.path`);
  if (!existsSync(artifactPath)) {
    fail(`${file}.artifact ${id} points at missing file ${artifactRef.path}`);
  }
  const digest = sha256File(artifactPath);
  assertDigest(artifactRef.digest, `${file}.artifact.digest`);
  if (digest !== artifactRef.digest) {
    fail(`${file}.artifact ${id} digest mismatch`);
  }
  if (artifactDigests[id] !== digest) {
    fail(`${file}.result.artifact_digests.${id} must match artifact file digest`);
  }
}

function validateArtifactGraph(fixtureRoot, rawById) {
  const referencedPaths = new Map();
  const artifactIds = new Set();
  for (const raw of rawById.values()) {
    for (const artifactRef of raw.result.artifact_refs) {
      if (artifactIds.has(artifactRef.id)) {
        fail(`duplicate artifact id ${artifactRef.id}`);
      }
      artifactIds.add(artifactRef.id);
      const current = referencedPaths.get(artifactRef.path);
      if (current) {
        fail(`artifact path ${artifactRef.path} is referenced by both ${current} and ${raw.experiment_id}`);
      }
      referencedPaths.set(artifactRef.path, raw.experiment_id);
    }
  }
  const artifactsDir = path.join(fixtureRoot, "artifacts");
  if (!existsSync(artifactsDir)) {
    return;
  }
  for (const file of collectFiles(artifactsDir)) {
    const relative = path.relative(fixtureRoot, file).split(path.sep).join("/");
    if (!referencedPaths.has(relative)) {
      fail(`orphan artifact file ${relative}`);
    }
    assertNoUnboundDigestsInArtifact(file, relative);
  }
}

function assertNoUnboundDigestsInArtifact(file, relative) {
  const text = readFileSync(file, "utf8");
  if (!DIGEST_SUBSTRING_PATTERN.test(text)) {
    return;
  }
  fail(`artifact ${relative} contains a nested digest claim without an artifact_ref`);
}

function collectFiles(dir) {
  const files = [];
  for (const name of readdirSyncSorted(dir)) {
    const file = path.join(dir, name);
    if (statSync(file).isDirectory()) {
      files.push(...collectFiles(file));
    } else {
      files.push(file);
    }
  }
  return files;
}

async function validateExpectedManifest(fixtureRoot, suite, rawById) {
  const expectedPath = path.join(fixtureRoot, "expected", "normalized-manifest.json");
  const expected = assertObject(readJson(expectedPath), expectedPath);
  assertOnlyKeys(
    expected,
    new Set(["schema_version", "payload_version", "suite_id", "schema_refs", "provenance_ref", "capability_instances"]),
    "expected",
  );
  assertMajorOne(expected.schema_version, EXPECTED_SCHEMA, "expected.schema_version");
  assertPayloadMajorOne(expected.payload_version, EXPECTED_PAYLOAD_PREFIX, "expected.payload_version");
  if (expected.suite_id !== suite.suite_id) {
    fail("expected.suite_id must match experiment suite");
  }
  const schemaRefs = loadSchemaRefs(fixtureRoot, expected);
  if (schemaRefs.raw.path !== RAW_SCHEMA_REF_PATH || schemaRefs.expected.path !== EXPECTED_SCHEMA_REF_PATH || schemaRefs.provenance.path !== PROVENANCE_SCHEMA_REF_PATH) {
    fail("expected.schema_refs must point at the versioned host-capability schemas");
  }
  const provenance = loadProvenance(fixtureRoot, expected.provenance_ref, schemaRefs);
  if (provenance.record.suite_id !== suite.suite_id) {
    fail("provenance.suite_id must match experiment suite");
  }
  const entries = assertArray(expected.capability_instances, "expected.capability_instances");
  if (entries.length !== rawById.size) {
    fail("expected.capability_instances must cover every raw capture exactly once");
  }
  const seen = new Set();
  for (const entry of entries) {
    await validateExpectedEntry(fixtureRoot, entry, rawById, provenance, schemaRefs);
    if (seen.has(entry.raw_capture_id)) {
      fail(`duplicate expected entry for ${entry.raw_capture_id}`);
    }
    seen.add(entry.raw_capture_id);
  }
  await runCanonicalValidator({
    raw_documents: [...rawById.values()],
    expected_document: expected,
    provenance_document: provenance.record,
    schemas: {
      raw: schemaRefs.raw.document,
      expected: schemaRefs.expected.document,
      provenance: schemaRefs.provenance.document,
    },
    capability_instances: entries.map((entry) => entry.capability_instance),
  });
  validateHostSurfaceMatrix(fixtureRoot, expected, rawById, provenance.record);
  for (const id of rawById.keys()) {
    if (!seen.has(id)) {
      fail(`missing expected entry for ${id}`);
    }
  }
  assertNoUnboundDigests(expected, "expected", (segments) => {
    return (
      isPath(segments, ["capability_instances", "*", "manifest_ref", "digest"]) ||
      isPath(segments, ["capability_instances", "*", "provenance_ref", "digest"]) ||
      isPath(segments, ["capability_instances", "*", "capability_instance", "manifest_digest"]) ||
      isPath(segments, ["capability_instances", "*", "capability_instance", "environment", "digest"]) ||
      isPath(segments, ["schema_refs", "raw", "digest"]) ||
      isPath(segments, ["schema_refs", "expected", "digest"]) ||
      isPath(segments, ["schema_refs", "provenance", "digest"]) ||
      isPath(segments, ["provenance_ref", "digest"])
    );
  });
  return expected;
}

function validateHostSurfaceMatrix(fixtureRoot, expected, rawById, provenanceRecord) {
  const matrixPath = path.join(fixtureRoot, "expected", "host-surface-matrix.json");
  if (!existsSync(matrixPath)) {
    fail("expected/host-surface-matrix.json is required");
  }
  const actual = readJson(matrixPath);
  const generated = hostSurfaceMatrixFromExpected(expected, rawById, provenanceRecord);
  if (stableJson(actual) !== stableJson(generated)) {
    fail("expected host-surface matrix drifted from verified manifests and fixtures");
  }
}

function hostSurfaceMatrixFromExpected(expected, rawById, provenanceRecord) {
  const provenanceById = new Map(provenanceRecord.captures.map((capture) => [capture.experiment_id, capture]));
  return {
    schema_version: "planr.host_surface_capability_matrix.v1",
    fixture_contract: "host-capability-raw/1.0.0",
    suite_id: expected.suite_id,
    surfaces: expected.capability_instances.map((entry) => {
      const instance = entry.capability_instance;
      const raw = rawById.get(entry.raw_capture_id);
      const provenance = provenanceById.get(entry.raw_capture_id);
      return {
        host: instance.host,
        surface: instance.surface,
        host_version: instance.host_version,
        trusted_adapter_enabled: entry.trusted_adapter_enabled,
        availability_status: instance.availability.status,
        reason: instance.availability.reason,
        observation_types: instance.observed_payload_contract.observation_types,
        provenance: {
          claim_source: entry.claim_source,
          source_kind: provenance.source_kind,
          observation_mode: provenance.observation_mode,
        },
        permissions: instance.permissions,
        artifact_kinds: raw.result.artifact_refs.map((artifact) => artifact.kind).sort(),
        blind_spots: instance.limitations,
      };
    }),
  };
}

async function validateExpectedEntry(fixtureRoot, entry, rawById, provenance, schemaRefs) {
  assertOnlyKeys(
    entry,
    new Set([
      "raw_capture_id",
      "claim_source",
      "trusted_adapter_enabled",
      "manifest_ref",
      "provenance_ref",
      "capability_instance",
    ]),
    "expected.capability_instances[]",
  );
  const rawCaptureId = assertNonEmptyString(entry.raw_capture_id, "entry.raw_capture_id");
  const raw = rawById.get(rawCaptureId);
  if (!raw) {
    fail(`expected entry references unknown raw capture ${rawCaptureId}`);
  }
  const claimSource = assertNonEmptyString(entry.claim_source, "entry.claim_source");
  if (raw.result.final_status === "available" && !OBSERVED_CLAIM_SOURCES.has(claimSource)) {
    fail(`available entry ${rawCaptureId} cannot be supported by docs-only claims`);
  }
  if (entry.trusted_adapter_enabled !== false) {
    fail(`entry ${rawCaptureId} must not enable trusted adapters in phase 1`);
  }
  if (!sameFileRef(entry.provenance_ref, provenance.ref)) {
    fail(`entry ${rawCaptureId} provenance_ref must match expected provenance_ref`);
  }
  const provenanceCapture = provenance.byId.get(rawCaptureId);
  if (!provenanceCapture) {
    fail(`missing provenance capture for ${rawCaptureId}`);
  }
  await validateProvenanceBinding(fixtureRoot, entry, raw, provenanceCapture);
  const manifestRef = validateManifestRef(fixtureRoot, entry.manifest_ref, rawCaptureId);
  const instance = assertObject(entry.capability_instance, "entry.capability_instance");
  validateEvidenceCapabilityInstanceShape(instance, raw, manifestRef, provenanceCapture, schemaRefs);
}

function validateManifestRef(fixtureRoot, manifestRef, rawCaptureId) {
  assertOnlyKeys(manifestRef, new Set(["path", "digest"]), "entry.manifest_ref");
  const file = safeFixturePath(fixtureRoot, manifestRef.path, "entry.manifest_ref.path");
  if (!existsSync(file)) {
    fail(`entry ${rawCaptureId} manifest_ref points at missing file`);
  }
  assertDigest(manifestRef.digest, "entry.manifest_ref.digest");
  const digest = sha256File(file);
  if (manifestRef.digest !== digest) {
    fail(`entry ${rawCaptureId} manifest_ref digest mismatch`);
  }
  const manifest = assertObject(readJson(file), manifestRef.path);
  assertOnlyKeys(
    manifest,
    new Set(["schema_version", "manifest_ids", "trusted_adapter_enabled", "source"]),
    "manifest_ref",
  );
  if (manifest.schema_version !== "planr.host_capability_manifest_reference.v1") {
    fail("manifest_ref.schema_version must be planr.host_capability_manifest_reference.v1");
  }
  const expectedManifestId = `host-${rawCaptureId.replace(/^exp-/, "")}-manifest`;
  if (!assertArray(manifest.manifest_ids, "manifest_ref.manifest_ids").includes(expectedManifestId)) {
    fail(`entry ${rawCaptureId} manifest_ref does not bind the manifest id`);
  }
  if (manifest.trusted_adapter_enabled !== false) {
    fail("manifest_ref must not enable trusted adapters in phase 1");
  }
  return manifestRef;
}

function validateEvidenceCapabilityInstanceShape(instance, raw, manifestRef, provenance, schemaRefs) {
  const required = new Set([
    "id",
    "schema_version",
    "manifest_id",
    "manifest_digest",
    "host",
    "surface",
    "host_version",
    "adapter_version",
    "environment",
    "permissions",
    "availability",
    "probe_result",
    "observed_payload_contract",
    "limitations",
    "captured_at",
  ]);
  assertOnlyKeys(instance, required, "capability_instance");
  for (const field of required) {
    if (!(field in instance)) {
      fail(`capability_instance missing ${field}`);
    }
  }
  if (instance.schema_version !== "evidence.contract.v1") {
    fail("capability_instance.schema_version must reuse Evidence contract v1");
  }
  assertNonEmptyString(instance.id, "capability_instance.id");
  assertNonEmptyString(instance.manifest_id, "capability_instance.manifest_id");
  assertDigest(instance.manifest_digest, "capability_instance.manifest_digest");
  if (instance.manifest_digest !== manifestRef.digest) {
    fail("capability_instance.manifest_digest must be content-bound by manifest_ref");
  }
  if (instance.host !== raw.host_identity.host) {
    fail("capability_instance.host must come from raw host identity");
  }
  if (instance.surface !== raw.surface) {
    fail("capability_instance.surface must come from raw capture");
  }
  if (instance.host_version !== raw.host_identity.version) {
    fail("capability_instance.host_version must come from raw host identity");
  }
  if (instance.adapter_version !== raw.host_identity.adapter_version) {
    fail("capability_instance.adapter_version must come from raw host identity");
  }
  validateEnvironmentBinding(instance.environment, "capability_instance.environment");
  validatePermissionState(instance.permissions, "capability_instance.permissions");
  if (stableJson(instance.permissions) !== stableJson(raw.result.permissions)) {
    fail("capability_instance.permissions must be projected from raw result permissions");
  }
  validateAvailability(instance.availability, raw, provenance);
  validateProbeResult(instance.probe_result, raw, instance.availability.status, provenance);
  validateObservedPayloadContract(instance.observed_payload_contract, schemaRefs);
  validateOptionalStringList(assertArray(instance.limitations, "capability_instance.limitations"), "capability_instance.limitations");
  if (stableJson(instance.limitations) !== stableJson(raw.result.blind_spots)) {
    fail("capability_instance.limitations must be projected from raw result blind_spots");
  }
  assertTimestamp(instance.captured_at, "capability_instance.captured_at");
  if (instance.captured_at !== raw.ended_at) {
    fail("capability_instance.captured_at must come from raw ended_at");
  }
}

function validateEnvironmentBinding(environment, label) {
  assertOnlyKeys(environment, new Set(["kind", "id", "digest"]), label);
  assertNonEmptyString(environment.kind, `${label}.kind`);
  assertNonEmptyString(environment.id, `${label}.id`);
  assertDigest(environment.digest, `${label}.digest`);
}

function validatePermissionState(permissions, label) {
  assertOnlyKeys(permissions, new Set(["network", "filesystem", "environment", "secrets"]), label);
  assertNonEmptyString(permissions.network, `${label}.network`);
  assertNonEmptyString(permissions.filesystem, `${label}.filesystem`);
  if (permissions.environment !== undefined) {
    assertNonEmptyString(permissions.environment, `${label}.environment`);
  }
  if (permissions.secrets !== undefined) {
    assertNonEmptyString(permissions.secrets, `${label}.secrets`);
  }
}

function validateAvailability(availability, raw, provenance) {
  assertOnlyKeys(availability, new Set(["status", "reason"]), "capability_instance.availability");
  const status = assertNonEmptyString(availability.status, "capability_instance.availability.status");
  if (!EVIDENCE_AVAILABILITY_STATUSES.has(status)) {
    fail("capability_instance.availability.status must reuse Evidence status vocabulary");
  }
  if (status !== raw.result.final_status) {
    fail("capability_instance.availability.status must be projected from raw result.final_status");
  }
  if (status !== "available" && availability.reason === undefined) {
    fail("capability_instance.availability.reason is required for non-available statuses");
  }
  if (provenance.availability_reason !== raw.result.availability_reason) {
    fail("provenance availability_reason must match raw result.availability_reason");
  }
  if (availability.reason !== undefined) {
    assertNonEmptyString(availability.reason, "capability_instance.availability.reason");
  }
  if (availability.reason !== provenance.availability_reason) {
    fail("capability_instance.availability.reason must be bound to raw and provenance");
  }
}

function validateProbeResult(probe, raw, availabilityStatus, provenance) {
  assertOnlyKeys(
    probe,
    new Set(["probe_execution_id", "outcome", "observed_at", "checks"]),
    "capability_instance.probe_result",
  );
  assertNonEmptyString(probe.probe_execution_id, "capability_instance.probe_result.probe_execution_id");
  const probeOutcome = assertNonEmptyString(probe.outcome, "capability_instance.probe_result.outcome");
  if (!EVIDENCE_ATTEMPT_STATUSES.has(probeOutcome)) {
    fail("capability_instance.probe_result.outcome must reuse Evidence attempt status vocabulary");
  }
  if (availabilityStatus === "available" && probeOutcome !== "passed") {
    fail("capability_instance.probe_result.outcome must pass when availability is available");
  }
  assertTimestamp(probe.observed_at, "capability_instance.probe_result.observed_at");
  if (probe.observed_at !== raw.ended_at) {
    fail("capability_instance.probe_result.observed_at must come from raw ended_at");
  }
  const checks = assertArray(probe.checks, "capability_instance.probe_result.checks");
  if (checks.length === 0) {
    fail("capability_instance.probe_result.checks must not be empty");
  }
  validateProbeChecks(checks, "capability_instance.probe_result.checks");
  if (stableJson(checks) !== stableJson(provenance.probe_checks)) {
    fail("capability_instance.probe_result.checks must be bound to provenance");
  }
}

function stableJson(value) {
  if (!value || typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableJson).join(",")}]`;
  }
  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
    .join(",")}}`;
}

function validateProbeChecks(checks, label) {
  const entries = assertArray(checks, label);
  if (entries.length === 0) {
    fail(`${label} must not be empty`);
  }
  for (const [index, check] of entries.entries()) {
    assertOnlyKeys(
      check,
      new Set(["name", "outcome", "detail"]),
      `${label}[${index}]`,
    );
    assertNonEmptyString(check.name, `${label}[${index}].name`);
    const outcome = assertNonEmptyString(check.outcome, `${label}[${index}].outcome`);
    if (!EVIDENCE_ATTEMPT_STATUSES.has(outcome)) {
      fail(`${label}[].outcome must reuse Evidence attempt status vocabulary`);
    }
    if (check.detail !== undefined) {
      assertNonEmptyString(check.detail, `${label}[${index}].detail`);
    }
  }
}

function validateObservedPayloadContract(payloadContract, schemaRefs) {
  assertOnlyKeys(
    payloadContract,
    new Set(["schema_ref", "observation_types"]),
    "capability_instance.observed_payload_contract",
  );
  assertNonEmptyString(payloadContract.schema_ref, "observed_payload_contract.schema_ref");
  if (payloadContract.schema_ref !== schemaRefs.raw.path) {
    fail("observed_payload_contract.schema_ref must point at the versioned raw schema");
  }
  const observationTypes = assertArray(payloadContract.observation_types, "observed_payload_contract.observation_types");
  if (observationTypes.length === 0) {
    fail("observed_payload_contract.observation_types must not be empty");
  }
  validateOptionalStringList(observationTypes, "observed_payload_contract.observation_types");
}

async function validateProvenanceBinding(fixtureRoot, entry, raw, provenance) {
  const rawCaptureId = entry.raw_capture_id;
  const rule = CLAIM_SOURCE_RULES[entry.claim_source];
  if (provenance.claim_source !== entry.claim_source) {
    fail(`entry ${rawCaptureId} claim_source must match provenance`);
  }
  if (provenance.source_kind !== rule.sourceKind) {
    fail(`entry ${rawCaptureId} claim_source is not valid for provenance source_kind`);
  }
  if (provenance.observation_mode !== rule.observationMode) {
    fail(`entry ${rawCaptureId} observation_mode is not valid for claim_source`);
  }
  if (!rule.inputKinds.has(raw.input.input_kind)) {
    fail(`entry ${rawCaptureId} claim_source is not valid for raw input_kind`);
  }
  if (entry.claim_source === "capture_mode_placeholder" && raw.input.replay_mode !== "unprobed-placeholder") {
    fail(`entry ${rawCaptureId} capture_mode_placeholder requires unprobed placeholder replay_mode`);
  }
  if (entry.claim_source === "mechanical_unavailable_probe") {
    validateMechanicalUnavailableProbe(fixtureRoot, rawCaptureId, raw, provenance);
  }
  if (!sameFileRef(raw.provenance_ref, entry.provenance_ref)) {
    fail(`entry ${rawCaptureId} raw provenance_ref must match expected entry`);
  }
  for (const [field, actual] of [
    ["host", raw.host_identity.host],
    ["surface", raw.surface],
    ["input_kind", raw.input.input_kind],
    ["tool_name", raw.tool_name],
    ["event_source", raw.event_source],
    ["host_version", raw.host_identity.version],
    ["adapter_version", raw.host_identity.adapter_version],
  ]) {
    if (provenance[field] !== actual) {
      fail(`entry ${rawCaptureId} provenance ${field} must match raw capture`);
    }
  }
  if (
    raw.host_identity.version === "current-session" ||
    raw.host_identity.version.endsWith("-current-session") ||
    raw.host_identity.version === "unavailable-in-fixture-replay"
  ) {
    fail(`entry ${rawCaptureId} host_version must be actual or explicitly missing`);
  }
  if (raw.host_identity.version === "missing" && !raw.result.missing_fields.includes("host_version")) {
    fail(`entry ${rawCaptureId} missing host_version must be declared in raw missing_fields`);
  }
  if (provenance.host_version === "missing" && !provenance.missing_fields.includes("host_version")) {
    fail(`entry ${rawCaptureId} missing host_version must be declared in provenance missing_fields`);
  }
  if (provenance.captured_at !== raw.ended_at) {
    fail(`entry ${rawCaptureId} provenance captured_at must match raw ended_at`);
  }
  const artifactIds = raw.result.artifact_refs.map((artifactRef) => artifactRef.id).sort();
  if (JSON.stringify([...provenance.artifact_ids].sort()) !== JSON.stringify(artifactIds)) {
    fail(`entry ${rawCaptureId} provenance artifact_ids must match raw artifact_refs`);
  }
  await validateClaimedObservationSupport(fixtureRoot, rawCaptureId, raw, provenance);
}

function validateMechanicalUnavailableProbe(fixtureRoot, rawCaptureId, raw, provenance) {
    if (raw.result.artifact_refs.length === 0 || provenance.artifact_ids.length === 0) {
      fail(`entry ${rawCaptureId} mechanical_unavailable_probe requires invocation artifacts`);
    }
    if (raw.result.final_status === "available") {
      fail(`entry ${rawCaptureId} mechanical_unavailable_probe must not claim available`);
    }
    if (provenance.external_producer !== undefined) {
      fail(`entry ${rawCaptureId} mechanical_unavailable_probe must not use external observed producer provenance`);
    }
    if (raw.host_identity.host !== "codex") {
      validatePeerVersionProbeArtifacts(fixtureRoot, rawCaptureId, raw, provenance);
      if (!raw.result.missing_fields.includes("final_result_payload")) {
        fail(`entry ${rawCaptureId} mechanical_unavailable_probe must declare missing final_result_payload`);
      }
    }
}

function validatePeerVersionProbeArtifacts(fixtureRoot, rawCaptureId, raw, provenance) {
  if (!Array.isArray(raw.input.command) || raw.input.command.length === 0) {
    fail(`entry ${rawCaptureId} mechanical_unavailable_probe requires exact argv command`);
  }
  const artifactsByKind = new Map(raw.result.artifact_refs.map((artifactRef) => [artifactRef.kind, artifactRef]));
  for (const kind of ["invocation-stdout", "invocation-stderr", "invocation-result"]) {
    if (!artifactsByKind.has(kind)) {
      fail(`entry ${rawCaptureId} mechanical_unavailable_probe missing ${kind} artifact`);
    }
  }
  const resultArtifact = readJson(safeFixturePath(fixtureRoot, artifactsByKind.get("invocation-result").path, "mechanical invocation result"));
  assertOnlyKeys(
    resultArtifact,
    new Set(["schema_version", "argv", "cwd", "exit_status", "signal", "stdout_bytes", "stderr_bytes"]),
    `entry ${rawCaptureId} invocation-result`,
  );
  if (resultArtifact.schema_version !== "planr.host_capability_mechanical_invocation.v1") {
    fail(`entry ${rawCaptureId} invocation-result schema_version is unsupported`);
  }
  if (stableJson(resultArtifact.argv) !== stableJson(raw.input.command)) {
    fail(`entry ${rawCaptureId} invocation-result argv must match raw input.command`);
  }
  if (!Number.isInteger(resultArtifact.exit_status)) {
    fail(`entry ${rawCaptureId} invocation-result exit_status is required`);
  }
  const stdoutText = readFileSync(safeFixturePath(fixtureRoot, artifactsByKind.get("invocation-stdout").path, "mechanical stdout"), "utf8");
  const stderrText = readFileSync(safeFixturePath(fixtureRoot, artifactsByKind.get("invocation-stderr").path, "mechanical stderr"), "utf8");
  if (Buffer.byteLength(stdoutText) !== resultArtifact.stdout_bytes) {
    fail(`entry ${rawCaptureId} invocation-result stdout_bytes must match stdout artifact`);
  }
  if (Buffer.byteLength(stderrText) !== resultArtifact.stderr_bytes) {
    fail(`entry ${rawCaptureId} invocation-result stderr_bytes must match stderr artifact`);
  }
  const version = parseVersionProbeStdout(raw.host_identity.host, stdoutText);
  if (!version) {
    fail(`entry ${rawCaptureId} mechanical_unavailable_probe could not parse host version from stdout artifact`);
  }
  if (raw.host_identity.version !== version || provenance.host_version !== version) {
    fail(`entry ${rawCaptureId} host_version must be derived from invocation stdout artifact`);
  }
}

async function validateClaimedObservationSupport(fixtureRoot, rawCaptureId, raw, provenance) {
  const reasons = [
    ["raw result.availability_reason", raw.result.availability_reason],
    ["provenance availability_reason", provenance.availability_reason],
  ];
  for (const [label, reason] of reasons) {
    const text = assertNonEmptyString(reason, `entry ${rawCaptureId} ${label}`);
    if (claimsScreenshotDigestObservation(text) && !(await hasContentBoundScreenshotArtifact(fixtureRoot, rawCaptureId, raw))) {
      fail(`entry ${rawCaptureId} ${label} claims screenshot digest observations without a content-bound screenshot artifact`);
    }
  }
}

function claimsScreenshotDigestObservation(text) {
  return /\bscreenshot\b/i.test(text) && /\b(?:digest|hash|checksum|sha[- ]?256)\b/i.test(text);
}

async function hasContentBoundScreenshotArtifact(fixtureRoot, rawCaptureId, raw) {
  for (const artifactRef of raw.result.artifact_refs) {
    if (!/\bscreenshot\b/i.test(artifactRef.kind)) {
      continue;
    }
    await validateContentBoundScreenshotArtifact(fixtureRoot, rawCaptureId, artifactRef);
    return true;
  }
  return false;
}

async function validateContentBoundScreenshotArtifact(fixtureRoot, rawCaptureId, artifactRef) {
  if (artifactRef.kind !== "screenshot") {
    fail(`entry ${rawCaptureId} screenshot artifact ${artifactRef.id} kind must be screenshot`);
  }
  const artifactPath = safeFixturePath(fixtureRoot, artifactRef.path, "screenshot artifact path");
  await validateScreenshotWithCanonicalValidator(
    artifactPath,
    artifactRef.digest,
    `entry ${rawCaptureId} screenshot artifact ${artifactRef.id}`,
  );
}

function sameFileRef(left, right) {
  return left?.path === right?.path && left?.digest === right?.digest;
}

function validateOptionalStringList(values, label) {
  for (const [index, value] of values.entries()) {
    assertNonEmptyString(value, `${label}[${index}]`);
  }
}

function isPath(segments, pattern) {
  if (segments.length !== pattern.length) {
    return false;
  }
  return pattern.every((part, index) => part === "*" || part === segments[index]);
}

function assertNoUnboundDigests(value, label, isAllowed, segments = []) {
  if (typeof value === "string") {
    if (DIGEST_SUBSTRING_PATTERN.test(value) && !isAllowed(segments)) {
      fail(`${label}.${segments.join(".")} contains a digest without an artifact or manifest ref`);
    }
    return;
  }
  if (!value || typeof value !== "object") {
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((entry) => assertNoUnboundDigests(entry, label, isAllowed, [...segments, "*"]));
    return;
  }
  for (const [key, entry] of Object.entries(value)) {
    assertNoUnboundDigests(entry, label, isAllowed, [...segments, key]);
  }
}

function validatorExecutableName() {
  return process.platform === "win32" ? `${VALIDATOR_NAME}.exe` : VALIDATOR_NAME;
}

function packagedValidatorPath() {
  const harnessPath = process.argv[1];
  if (!harnessPath) {
    fail("canonical Rust capability validator requires a harness executable path");
  }
  return path.resolve(path.dirname(harnessPath), validatorExecutableName());
}

function resolveCanonicalValidator() {
  const configured = process.env.PLANR_HOST_CAPABILITY_VALIDATOR;
  if (configured) {
    if (!path.isAbsolute(configured)) {
      fail(`canonical Rust capability validator path must be absolute: ${configured}`);
    }
    if (!existsSync(configured)) {
      fail(`canonical Rust capability validator path points at a missing file: ${configured}`);
    }
    return configured;
  }
  const packagedSibling = packagedValidatorPath();
  if (existsSync(packagedSibling)) {
    return packagedSibling;
  }
  fail(
    `canonical Rust capability validator not found at packaged sibling ${packagedSibling}; set PLANR_HOST_CAPABILITY_VALIDATOR to an absolute built ${VALIDATOR_NAME} binary or install it alongside the harness executable`,
  );
}

async function validateScreenshotWithCanonicalValidator(artifactPath, expectedDigest, label) {
  assertDigest(expectedDigest, `${label}.digest`);
  const validatorPath = resolveCanonicalValidator();
  const timeout = Number(process.env.PLANR_HOST_CAPABILITY_VALIDATOR_TIMEOUT_MS ?? "120000");
  const stdout = await spawnValidator(
    validatorPath,
    ["--validate-screenshot", artifactPath],
    timeout,
    `${label} canonical Rust screenshot validation`,
  );
  const result = parseValidatorJson(stdout, `${label} screenshot validator result`);
  assertValidatorResult(result, { inputDigest: expectedDigest, rawDocuments: 0, instances: 0 });
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function killValidatorProcessTree(child, signal) {
  if (!child.pid) {
    return;
  }
  try {
    if (process.platform === "win32") {
      const force = signal === "SIGKILL" ? ["/f"] : [];
      spawnSync("taskkill", ["/pid", String(child.pid), "/t", ...force], {
        encoding: "utf8",
        stdio: "ignore",
      });
      return;
    }
    process.kill(-child.pid, signal);
  } catch (error) {
    if (error?.code !== "ESRCH") {
      throw error;
    }
  }
}

async function terminateValidatorProcessTree(child) {
  killValidatorProcessTree(child, "SIGTERM");
  await sleep(100);
  killValidatorProcessTree(child, "SIGKILL");
}

async function spawnValidator(validatorPath, args, timeout, label) {
  const run = await new Promise((resolve) => {
    const child = spawn(validatorPath, args, {
      cwd: process.cwd(),
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    let spawnError = null;
    let directStatus = null;
    let directSignal = null;
    let cleanup = null;
    let resolved = false;
    const cleanupOnce = () => {
      cleanup ??= terminateValidatorProcessTree(child).catch((error) => {
        spawnError ??= error;
      });
      return cleanup;
    };
    const finish = async (status, signal) => {
      if (resolved) {
        return;
      }
      resolved = true;
      clearTimeout(timer);
      await (cleanup ?? Promise.resolve());
      resolve({
        error: spawnError,
        status: directStatus ?? status,
        signal: directSignal ?? signal,
        stdout,
        stderr,
        timedOut,
      });
    };
    const timer = setTimeout(() => {
      timedOut = true;
      cleanupOnce();
    }, timeout);
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (error) => {
      spawnError = error;
      clearTimeout(timer);
      cleanupOnce();
      setImmediate(() => {
        finish(null, null);
      });
    });
    child.on("exit", (status, signal) => {
      directStatus = status;
      directSignal = signal;
      clearTimeout(timer);
      cleanupOnce();
    });
    child.on("close", async (status, signal) => {
      await finish(status, signal);
    });
  });
  if (run.error) {
    fail(`${label} failed to execute: ${run.error.message}`);
  }
  if (run.timedOut) {
    fail(`${label} failed to execute: timed out after ${timeout}ms`);
  }
  if (run.status !== 0) {
    const detail = ((run.stderr ?? "").trim() || (run.stdout ?? "").trim());
    fail(`${label} failed: ${detail || `status ${run.status} signal ${run.signal ?? "none"}`}`);
  }
  const stderr = (run.stderr ?? "").trim();
  if (stderr.length > 0) {
    fail(`${label} wrote stderr: ${stderr}`);
  }
  return run.stdout ?? "";
}

function parseValidatorJson(stdout, label) {
  const trimmed = stdout.trim();
  if (trimmed.length === 0) {
    fail(`${label} produced empty stdout`);
  }
  try {
    return assertObject(JSON.parse(trimmed), label);
  } catch (error) {
    fail(`${label} produced malformed JSON: ${error.message}`);
  }
}

function assertExactKeys(value, allowed, label) {
  assertOnlyKeys(value, allowed, label);
  for (const key of allowed) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      fail(`${label}.${key} is required`);
    }
  }
}

function assertValidatorIdentity(identity) {
  assertExactKeys(identity, VALIDATOR_IDENTITY_FIELDS, "validator identity");
  if (identity.schema_version !== VALIDATOR_IDENTITY_SCHEMA) {
    fail(`validator identity.schema_version must be ${VALIDATOR_IDENTITY_SCHEMA}`);
  }
  if (identity.validator !== VALIDATOR_NAME) {
    fail(`validator identity.validator must be ${VALIDATOR_NAME}`);
  }
  if (identity.validator_version !== VALIDATOR_VERSION) {
    fail(`validator identity.validator_version must be ${VALIDATOR_VERSION}`);
  }
}

function assertValidatorResult(result, expected) {
  assertExactKeys(result, VALIDATOR_RESULT_FIELDS, "validator result");
  if (result.schema_version !== VALIDATOR_RESULT_SCHEMA) {
    fail(`validator result.schema_version must be ${VALIDATOR_RESULT_SCHEMA}`);
  }
  if (result.validator !== VALIDATOR_NAME) {
    fail(`validator result.validator must be ${VALIDATOR_NAME}`);
  }
  if (result.validator_version !== VALIDATOR_VERSION) {
    fail(`validator result.validator_version must be ${VALIDATOR_VERSION}`);
  }
  if (result.verdict !== "pass") {
    fail("validator result.verdict must be pass");
  }
  if (result.input_digest !== expected.inputDigest) {
    fail("validator result.input_digest must match submitted validation bundle");
  }
  if (result.validated_raw_documents !== expected.rawDocuments) {
    fail("validator result.validated_raw_documents must match submitted raw document count");
  }
  if (result.validated_instances !== expected.instances) {
    fail("validator result.validated_instances must match submitted capability instance count");
  }
}

async function runCanonicalValidator(payload) {
  const rawDocuments = assertArray(payload.raw_documents, "validator payload.raw_documents").length;
  const instances = assertArray(payload.capability_instances, "validator payload.capability_instances").length;
  const validatorPath = resolveCanonicalValidator();
  const timeout = Number(process.env.PLANR_HOST_CAPABILITY_VALIDATOR_TIMEOUT_MS ?? "120000");
  const identity = parseValidatorJson(
    await spawnValidator(validatorPath, ["--identity"], timeout, "canonical Rust capability validator identity probe"),
    "validator identity",
  );
  assertValidatorIdentity(identity);
  const tempDir = mkdtempSync(path.join(os.tmpdir(), "planr-host-capability-validator-"));
  const inputPath = path.join(tempDir, "input.json");
  const inputBytes = `${JSON.stringify(payload)}\n`;
  const inputDigest = sha256Prefixed(inputBytes);
  writeFileSync(inputPath, inputBytes);
  try {
    const result = parseValidatorJson(
      await spawnValidator(validatorPath, ["--input", inputPath], timeout, "canonical Rust capability validation"),
      "validator result",
    );
    assertValidatorResult(result, { inputDigest, rawDocuments, instances });
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

async function buildReplaySummary(fixtureRoot) {
  const { suite, experiments } = loadSuite(fixtureRoot);
  const rawById = loadRawCaptures(fixtureRoot, experiments);
  const expected = await validateExpectedManifest(fixtureRoot, suite, rawById);
  return {
    schema_version: SUMMARY_SCHEMA,
    verdict: "pass",
    suite_id: suite.suite_id,
    fixture_root: fixtureRoot,
    experiment_count: experiments.size,
    availability: Object.fromEntries(
      expected.capability_instances.map((entry) => [
        entry.raw_capture_id,
        {
          availability_status: entry.capability_instance.availability.status,
          trusted_adapter_enabled: entry.trusted_adapter_enabled,
        },
      ]),
    ),
  };
}

async function replay(fixtureRoot) {
  const summary = await buildReplaySummary(fixtureRoot);
  console.log(JSON.stringify(summary));
}

function copyDirectory(source, target) {
  mkdirSync(target, { recursive: true });
  for (const entry of readdirSync(source, { withFileTypes: true })) {
    const from = path.join(source, entry.name);
    const to = path.join(target, entry.name);
    if (entry.isDirectory()) {
      copyDirectory(from, to);
    } else if (entry.isFile()) {
      copyFileSync(from, to);
    } else {
      fail(`import fixture contains unsupported filesystem entry ${from}`);
    }
  }
}

function isSameOrInside(child, parent) {
  const relative = path.relative(parent, child);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function assertNoSymlinkPath(targetPath, label = "path") {
  const resolved = path.resolve(targetPath);
  let current = path.parse(resolved).root;
  for (const segment of path.relative(current, resolved).split(path.sep)) {
    if (!segment) continue;
    current = path.join(current, segment);
    if (existsSync(current) && lstatSync(current).isSymbolicLink()) {
      fail(`${label} must not traverse symlink component ${current}`);
    }
  }
}

function validateOutputDestination(outDir, options = {}) {
  assertNonEmptyString(outDir, "capture --out-dir");
  const destination = path.resolve(outDir);
  const parent = path.dirname(destination);
  assertNoSymlinkPath(destination, "capture --out-dir");
  if (!existsSync(parent)) {
    fail(`capture --out-dir parent must already exist: ${parent}`);
  }
  if (existsSync(destination) && lstatSync(destination).isSymbolicLink()) {
    fail("capture --out-dir must not be a symlink");
  }
  const parentReal = realpathSync(parent);
  const destinationReal = existsSync(destination) ? realpathSync(destination) : path.join(parentReal, path.basename(destination));
  const repoRoot = realpathSync(packageRoot());
  const protectedRuntimeRoots = [
    ["host capability runtime root", packagedRuntimeRoot()],
    ["host capability fixture root", packagedFixtureRoot()],
  ].filter(([, protectedRoot]) => existsSync(protectedRoot));
  for (const [label, protectedRoot] of [
    ["repository root", repoRoot],
    ...protectedRuntimeRoots.map(([label, protectedRoot]) => [label, realpathSync(protectedRoot)]),
  ]) {
    if (destinationReal === protectedRoot || isSameOrInside(protectedRoot, destinationReal)) {
      fail(`capture --out-dir must not target ${label} or one of its ancestors`);
    }
  }
  if (isSameOrInside(destinationReal, repoRoot)) {
    fail("capture --out-dir must not be inside the repository workspace");
  }
  if (options.importRoot) {
    assertNoSymlinkPath(options.importRoot, "--import-fixture-root");
    const importReal = realpathSync(options.importRoot);
    if (destinationReal === importReal || isSameOrInside(destinationReal, importReal) || isSameOrInside(importReal, destinationReal)) {
      fail("capture --out-dir must not overlap --import-fixture-root");
    }
  }
  if (existsSync(destination)) {
    if (!lstatSync(destination).isDirectory()) {
      fail("capture --out-dir must be an absent or empty directory");
    }
    if (readdirSync(destination).length !== 0) {
      fail("capture --out-dir must be empty before capture");
    }
  }
  return { destination, parent: parentReal };
}

async function generateAndPublish(outDir, options, generator) {
  const destination = validateOutputDestination(outDir, options);
  const staging = mkdtempSync(path.join(destination.parent, ".planr-host-capability-"));
  try {
    await generator(staging);
    validateOutputDestination(outDir, options);
    if (existsSync(destination.destination)) {
      rmdirSync(destination.destination);
    }
    renameSync(staging, destination.destination);
  } catch (error) {
    rmSync(staging, { recursive: true, force: true });
    throw error;
  }
}

function loadSchemaReferencesFromFiles(fixtureRoot) {
  const refs = {
    raw: { path: RAW_SCHEMA_REF_PATH },
    expected: { path: EXPECTED_SCHEMA_REF_PATH },
    provenance: { path: PROVENANCE_SCHEMA_REF_PATH },
  };
  for (const ref of Object.values(refs)) {
    const file = safeFixturePath(fixtureRoot, ref.path, "schema ref path");
    if (!existsSync(file)) {
      fail(`observed import bundle is missing schema file ${ref.path}`);
    }
    ref.digest = sha256File(file);
    ref.document = readJson(file);
  }
  return refs;
}

function loadAnchoredSuite(suite = defaultSuite()) {
  return { suite, experiments: new Map(suite.experiments.map((experiment) => [experiment.id, experiment])) };
}

function validateImportRoot(importRoot) {
  assertNonEmptyString(importRoot, "--import-fixture-root");
  assertNoSymlinkPath(importRoot, "--import-fixture-root");
  const resolved = path.resolve(importRoot);
  if (!existsSync(resolved) || !lstatSync(resolved).isDirectory()) {
    fail("--import-fixture-root must be an existing directory");
  }
  return resolved;
}

function loadExternalEnvelope(importRoot) {
  const root = validateImportRoot(importRoot);
  const envelopePath = safeFixturePath(root, EXTERNAL_CAPTURE_ENVELOPE_PATH, "external envelope path");
  const envelope = assertObject(readJson(envelopePath), envelopePath);
  assertOnlyKeys(envelope, new Set(["schema_version", "producer", "suite_id", "captures"]), "external envelope");
  assertMajorOne(envelope.schema_version, EXTERNAL_CAPTURE_ENVELOPE_SCHEMA, "external envelope.schema_version");
  assertNonEmptyString(envelope.suite_id, "external envelope.suite_id");
  assertOnlyKeys(
    envelope.producer,
    new Set(["name", "version", "captured_at"]),
    "external envelope.producer",
  );
  assertNonEmptyString(envelope.producer.name, "external envelope.producer.name");
  assertNonEmptyString(envelope.producer.version, "external envelope.producer.version");
  assertTimestamp(envelope.producer.captured_at, "external envelope.producer.captured_at");
  return { envelope, envelopeDigest: sha256File(envelopePath), root };
}

function loadExternalCaptures(importRoot, suite = defaultSuite()) {
  const { experiments } = loadAnchoredSuite(suite);
  const { envelope, envelopeDigest, root } = loadExternalEnvelope(importRoot);
  if (envelope.suite_id !== suite.suite_id) {
    fail("external envelope suite_id must match the harness-owned suite");
  }
  const rawById = new Map();
  const producer = {
    name: envelope.producer.name,
    version: envelope.producer.version,
    captured_at: envelope.producer.captured_at,
    envelope_digest: envelopeDigest,
  };
  const producedAt = parseUtcTimestamp(envelope.producer.captured_at, "external envelope.producer.captured_at");
  const now = Date.now();
  if (producedAt > now + EXTERNAL_CAPTURE_FUTURE_SKEW_MS) {
    fail("external envelope.producer.captured_at must not be in the future");
  }
  if (now - producedAt > EXTERNAL_CAPTURE_MAX_AGE_MS) {
    fail("external envelope.producer.captured_at is stale for fresh external import");
  }
  for (const [index, capture] of assertArray(envelope.captures, "external envelope.captures").entries()) {
    const raw = structuredClone(assertObject(capture, `external envelope.captures[${index}]`));
    delete raw.provenance_ref;
    const experimentId = assertExperimentId(raw.experiment_id, `external envelope.captures[${index}].experiment_id`);
    const anchored = experiments.get(experimentId);
    if (!anchored) {
      fail(`external envelope.captures[${index}] references unknown experiment ${experimentId}`);
    }
    if (rawById.has(experimentId)) {
      fail(`duplicate external capture for ${experimentId}`);
    }
    const { ended } = assertTimeRange(raw.started_at, raw.ended_at, `external envelope.captures[${index}]`);
    if (producedAt < ended) {
      fail("external envelope.producer.captured_at must not be earlier than included captures");
    }
    if (now - ended > EXTERNAL_CAPTURE_MAX_AGE_MS) {
      fail(`external capture ${anchored.id} ended_at is stale for fresh external import`);
    }
    const adapted = new Map([
      [
        anchored.id,
        {
          ...anchored,
          input_kind: raw.input?.input_kind,
          expected_result_status: raw.result?.final_status,
        },
      ],
    ]);
    validateRawCapture(root, raw, adapted, `external envelope.captures[${index}]`);
    if (raw.input.input_kind === "unprobed_placeholder") {
      fail(`external capture ${anchored.id} must be observed host output, not a placeholder`);
    }
    Object.defineProperty(raw, IMPORT_METADATA, {
      value: { producer },
      enumerable: false,
    });
    rawById.set(anchored.id, raw);
  }
  validateArtifactGraph(root, rawById);
  return { suite, captures: [...rawById.values()], producer };
}

async function importFixtureCapture(outDir, importFixtureRoot, suite = defaultSuite()) {
  await generateAndPublish(
    outDir,
    { importRoot: importFixtureRoot },
    async (stagingRoot) => {
      const { captures, producer } = loadExternalCaptures(importFixtureRoot, suite);
      writeSchemaReferences(stagingRoot);
      copyDirectory(path.join(importFixtureRoot, "artifacts"), path.join(stagingRoot, "artifacts"));
      for (const capture of captures) {
        const anchored = suite.experiments.find((experiment) => experiment.id === capture.experiment_id);
        writeJson(safeFixturePath(stagingRoot, `observed/${anchored.id}.json`, "observed capture path"), capture);
      }
      const allCaptures = completeCapturesWithUnavailablePlaceholders(
        suite,
        captures,
        stagingRoot,
        producer.captured_at,
      );
      writeJson(path.join(stagingRoot, "experiment-suite.json"), suiteFromCaptures(suite, allCaptures));
      for (const capture of allCaptures) {
        if (!existsSync(path.join(stagingRoot, "observed", `${capture.experiment_id}.json`))) {
          const anchored = suite.experiments.find((experiment) => experiment.id === capture.experiment_id);
          writeJson(safeFixturePath(stagingRoot, `observed/${anchored.id}.json`, "observed capture path"), capture);
        }
      }
      const schemaRefs = loadSchemaReferencesFromFiles(stagingRoot);
      const manifestRef = writeManifestReference(stagingRoot, allCaptures);
      const provenanceRef = writeProvenanceReference(stagingRoot, allCaptures, schemaRefs);
      for (const capture of allCaptures) {
        const anchored = suite.experiments.find((experiment) => experiment.id === capture.experiment_id);
        const rawPath = safeFixturePath(stagingRoot, `observed/${anchored.id}.json`, "observed capture path");
        const raw = readJson(rawPath);
        raw.provenance_ref = provenanceRef;
        writeJson(rawPath, raw);
      }
      const expected = expectedFromCaptures(allCaptures, manifestRef, schemaRefs, provenanceRef);
      writeJson(path.join(stagingRoot, "expected", "normalized-manifest.json"), expected);
      writeJson(
        path.join(stagingRoot, "expected", "host-surface-matrix.json"),
        hostSurfaceMatrixFromExpected(
          expected,
          new Map(allCaptures.map((capture) => [capture.experiment_id, capture])),
          readJson(path.join(stagingRoot, PROVENANCE_REF_PATH)),
        ),
      );
      await buildReplaySummary(stagingRoot);
    },
  );
  await replay(outDir);
}

function claimSourceForCapture(capture) {
  if (capture.input.input_kind === "unprobed_placeholder") return "capture_mode_placeholder";
  if (capture.input.input_kind === "mechanical_availability_probe") return "mechanical_unavailable_probe";
  return "observed_capture";
}

function sourceKindForCapture(capture) {
  const claimSource = claimSourceForCapture(capture);
  return CLAIM_SOURCE_RULES[claimSource].sourceKind;
}

function observationModeForCapture(capture) {
  const claimSource = claimSourceForCapture(capture);
  return CLAIM_SOURCE_RULES[claimSource].observationMode;
}

function probeOutcomeForCapture(capture) {
  if (capture.result.final_status === "available") return "passed";
  if (capture.result.final_status === "degraded") return "inconclusive";
  return "unavailable";
}

function completeCapturesWithUnavailablePlaceholders(suite, captures, fixtureRoot, placeholderTimestamp) {
  const byId = new Map(captures.map((capture) => [capture.experiment_id, capture]));
  return suite.experiments.map((experiment) => {
    const imported = byId.get(experiment.id);
    if (imported) return imported;
    if (experiment.input_kind === "mechanical_availability_probe") {
      return mechanicalUnavailableProbeCapture(fixtureRoot, experiment, placeholderTimestamp);
    }
    return unavailableCapture(experiment, placeholderTimestamp);
  });
}

function suiteFromCaptures(baseSuite, captures) {
  const byId = new Map(captures.map((capture) => [capture.experiment_id, capture]));
  return {
    ...baseSuite,
    experiments: baseSuite.experiments.map((experiment) => {
      const capture = byId.get(experiment.id);
      if (!capture) return experiment;
      return {
        ...experiment,
        input_kind: capture.input.input_kind,
        expected_result_status: capture.result.final_status,
      };
    }),
  };
}

export async function capture(outDir, options = {}, suite = defaultSuite()) {
  if (options.importFixtureRoot) {
    await importFixtureCapture(outDir, options.importFixtureRoot, suite);
    return;
  }

  await generateAndPublish(outDir, {}, async (stagingRoot) => {
    await captureInto(stagingRoot, suite);
  });
  await buildReplaySummary(outDir);
}

async function captureInto(outDir, suite = defaultSuite()) {
  const now = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  const nodeVersion = process.version;
  const terminal = spawnSync(process.execPath, ["-e", "process.stdout.write('planr-host-experiment-v1\\n')"], {
    encoding: "utf8",
  });
  const terminalStdout = Buffer.from(terminal.stdout ?? "", "utf8");
  function fixtureLocalFunction(left, right) {
    return { result: "ok", value: left + right };
  }
  const localValue = Buffer.from(`${JSON.stringify(fixtureLocalFunction(40, 2))}\n`, "utf8");
  mkdirSync(path.join(outDir, "artifacts", "terminal"), { recursive: true });
  mkdirSync(path.join(outDir, "artifacts", "local-function"), { recursive: true });
  writeFileSync(path.join(outDir, "artifacts", "terminal", "stdout.txt"), terminalStdout);
  writeFileSync(path.join(outDir, "artifacts", "local-function", "result.json"), localValue);
  writeJson(path.join(outDir, "experiment-suite.json"), suite);
  const captures = suite.experiments.map((experiment) => {
    if (experiment.id === "exp-terminal") {
      return availableCapture({
        fixtureRoot: outDir,
        id: experiment.id,
        host: experiment.host,
        surface: experiment.surface,
        version: nodeVersion,
        toolName: experiment.expected_tool_name,
        eventSource: experiment.expected_event_source,
        input: { command: [process.execPath, "-e", "process.stdout.write('planr-host-experiment-v1\\n')"] },
        artifactPath: "artifacts/terminal/stdout.txt",
        artifactId: "artifact-terminal-stdout",
        artifactKind: "stdout",
        finalStatus: terminal.status === 0 ? "available" : "probe_failed",
        startedAt: now,
        endedAt: now,
      });
    }
    if (experiment.id === "exp-local-function") {
      return availableCapture({
        fixtureRoot: outDir,
        id: experiment.id,
        host: experiment.host,
        surface: experiment.surface,
        version: nodeVersion,
        toolName: experiment.expected_tool_name,
        eventSource: experiment.expected_event_source,
        input: { function: "fixtureLocalFunction", args: [40, 2] },
        artifactPath: "artifacts/local-function/result.json",
        artifactId: "artifact-local-function-result",
        artifactKind: "json-result",
        finalStatus: "available",
        startedAt: now,
        endedAt: now,
      });
    }
    if (experiment.input_kind === "mechanical_availability_probe") {
      return mechanicalUnavailableProbeCapture(outDir, experiment, now);
    }
    return unavailableCapture(experiment, now);
  });
  writeJson(path.join(outDir, "experiment-suite.json"), suiteFromCaptures(suite, captures));
  const schemaRefs = writeSchemaReferences(outDir);
  const manifestRef = writeManifestReference(outDir, captures);
  const provenanceRef = writeProvenanceReference(outDir, captures, schemaRefs);
  for (const capture of captures) {
    capture.provenance_ref = provenanceRef;
    writeJson(path.join(outDir, "observed", `${capture.experiment_id}.json`), capture);
  }
  const expected = expectedFromCaptures(captures, manifestRef, schemaRefs, provenanceRef);
  writeJson(path.join(outDir, "expected", "normalized-manifest.json"), expected);
  writeJson(
    path.join(outDir, "expected", "host-surface-matrix.json"),
    hostSurfaceMatrixFromExpected(
      expected,
      new Map(captures.map((capture) => [capture.experiment_id, capture])),
      readJson(path.join(outDir, PROVENANCE_REF_PATH)),
    ),
  );
  await replay(outDir);
}

export function defaultSuite() {
  return {
    schema_version: SUITE_SCHEMA,
    suite_id: "codex-host-capability-phase1",
    docs_are_experiment_design_only: true,
    experiments: [
      experiment("exp-terminal", "terminal", "node:child_process.spawnSync", "node:child_process", "available"),
      experiment("exp-local-function", "local-function", "planr:local-function-fixture", "node:function-call", "available"),
      experiment(
        "exp-codex-app-server",
        "codex-app-server",
        "codex app-server",
        "process:codex",
        "unavailable",
        "unprobed_placeholder",
      ),
      experiment("exp-codex-exec", "codex-exec", "codex exec", "process:codex", "unavailable", "unprobed_placeholder"),
      experiment(
        "exp-codex-mcp-server",
        "codex-mcp-server",
        "codex mcp-server",
        "process:codex",
        "unavailable",
        "unprobed_placeholder",
      ),
      experiment("exp-mcp-browser", "mcp-browser", "mcp browser tool", "mcp:browser", "unavailable", "unprobed_placeholder"),
      experiment(
        "exp-built-in-browser",
        "built-in-browser",
        "browser tool",
        "codex:built-in-tool",
        "unavailable",
        "unprobed_placeholder",
      ),
      experiment("exp-chrome-cdp", "chrome-cdp", "chrome/cdp tool", "cdp:chrome", "unavailable", "unprobed_placeholder"),
      experiment(
        "exp-chrome-browser-client",
        "chrome-browser-client",
        "browser-client.mjs chrome Runtime.evaluate",
        "browser-client:chrome",
        "unavailable",
        "unprobed_placeholder",
      ),
      experiment(
        "exp-codex-hook-events",
        "codex-hook-events",
        "codex hook event stream",
        "codex:hook",
        "unavailable",
        "unprobed_placeholder",
      ),
      experiment(
        "exp-computer-use",
        "computer-use",
        "computer use tool",
        "codex:computer-use",
        "unavailable",
        "unprobed_placeholder",
      ),
      hostExperiment(
        "exp-claude-code-host-capture",
        "claude",
        "claude-code",
        "$HOME/.local/bin/claude --version",
        "process:claude-code",
        "unavailable",
        "mechanical_availability_probe",
      ),
      hostExperiment(
        "exp-cursor-agent-host-capture",
        "cursor",
        "cursor-agent",
        "$HOME/.local/bin/cursor-agent --version",
        "process:cursor-agent",
        "unavailable",
        "mechanical_availability_probe",
      ),
      hostExperiment(
        "exp-pi-cli-host-capture",
        "pi",
        "pi-cli",
        "$HOME/.local/bin/pi --version",
        "process:pi-cli",
        "unavailable",
        "mechanical_availability_probe",
      ),
    ],
  };
}

function experiment(id, surface, toolName, eventSource, status, inputKind = "controlled_probe") {
  return hostExperiment(id, "codex", surface, toolName, eventSource, status, inputKind);
}

function hostExperiment(id, host, surface, toolName, eventSource, status, inputKind = "controlled_probe") {
  return {
    id,
    host,
    surface,
    input_kind: inputKind,
    expected_tool_name: toolName,
    expected_event_source: eventSource,
    expected_result_status: status,
  };
}

function availableCapture(input) {
  const artifactDigest = sha256File(path.join(input.fixtureRoot, input.artifactPath));
  return {
    schema_version: RAW_SCHEMA,
    payload_version: `${RAW_PAYLOAD_PREFIX}1.0.0`,
    experiment_id: input.id,
    host_identity: {
      host: input.host,
      surface: input.surface,
      version: input.version,
      adapter_version: ADAPTER_VERSION,
    },
    surface: input.surface,
    tool_name: input.toolName,
    event_source: input.eventSource,
    started_at: input.startedAt,
    ended_at: input.endedAt,
    input: { input_kind: "controlled_probe", ...input.input },
    events: [
      {
        sequence: 1,
        event_name: "started",
        payload_version: "host-event/1.0.0",
        tool_name: input.toolName,
        event_source: input.eventSource,
        payload: { input_kind: "controlled_probe" },
      },
      {
        sequence: 2,
        event_name: "final",
        final: true,
        payload_version: "host-event/1.0.0",
        tool_name: input.toolName,
        event_source: input.eventSource,
        payload: { final_status: input.finalStatus },
      },
    ],
    result: {
      final_status: input.finalStatus,
      availability_reason: "controlled fixture probe passed",
      permissions: {
        network: "not_used",
        filesystem: "fixture-controlled",
        environment: "node process environment visible to controlled probe",
        secrets: "not_requested",
      },
      sandbox: { mode: "developer-local", writable_roots: ["fixture_root"] },
      missing_fields: [],
      blind_spots: ["agent transcript and private profile data intentionally not captured"],
      artifact_refs: [
        {
          id: input.artifactId,
          kind: input.artifactKind,
          root_kind: "fixture_root",
          path: input.artifactPath,
          digest: artifactDigest,
        },
      ],
      artifact_digests: { [input.artifactId]: artifactDigest },
    },
  };
}

function mechanicalUnavailableProbeCapture(fixtureRoot, experiment, now) {
  const argv = experiment.expected_tool_name.split(" ");
  const artifactDir = path.join("artifacts", experiment.id.replace(/^exp-/, ""));
  const probeTmp = path.join(fixtureRoot, artifactDir, "probe-tmp");
  mkdirSync(probeTmp, { recursive: true });
  const result = spawnSync(argv[0], argv.slice(1), {
    cwd: packageRoot(),
    env: { ...process.env, TMPDIR: probeTmp },
    encoding: "buffer",
    timeout: 5000,
    maxBuffer: 1024 * 1024,
  });
  rmSync(probeTmp, { recursive: true, force: true });
  const stdout = Buffer.from(result.stdout ?? "");
  const stderr = Buffer.from(result.stderr ?? "");
  const stdoutText = stdout.toString("utf8");
  const version = result.status === 0 ? parseVersionProbeStdout(experiment.host, stdoutText) : null;
  if (!Number.isInteger(result.status) || result.status !== 0 || result.signal !== null || !version) {
    rmSync(path.join(fixtureRoot, artifactDir), { recursive: true, force: true });
    const placeholder = unavailableCapture(
      { ...experiment, input_kind: "unprobed_placeholder" },
      now,
    );
    placeholder.result.availability_reason =
      `${experiment.host} ${experiment.surface} version probe was unavailable or unparsable; ` +
      "host capture downgraded to a missing-version placeholder";
    placeholder.result.experiment_plan = [
      `Repair or install the bounded read-only version probe ${argv.join(" ")}.`,
      "Re-run capture mode and require a successful, parseable mechanical invocation before recording host version provenance.",
      `Keep ${experiment.host} ${experiment.surface} host capture unavailable until a final event and artifact contract is observed.`,
    ];
    placeholder.result.notes = [
      "failed or unparsable version probes are not retained as mechanical_unavailable_probe provenance",
    ];
    return placeholder;
  }
  mkdirSync(path.join(fixtureRoot, artifactDir), { recursive: true });
  const stdoutPath = path.join(artifactDir, "stdout.txt").split(path.sep).join("/");
  const stderrPath = path.join(artifactDir, "stderr.txt").split(path.sep).join("/");
  const resultPath = path.join(artifactDir, "result.json").split(path.sep).join("/");
  const resultPayload = {
    schema_version: "planr.host_capability_mechanical_invocation.v1",
    argv,
    cwd: packageRoot(),
    exit_status: result.status,
    signal: result.signal,
    stdout_bytes: stdout.length,
    stderr_bytes: stderr.length,
  };
  writeFileSync(path.join(fixtureRoot, stdoutPath), stdout);
  writeFileSync(path.join(fixtureRoot, stderrPath), stderr);
  writeJson(path.join(fixtureRoot, resultPath), resultPayload);
  const artifactRefs = [
    artifactRef(fixtureRoot, `artifact-${experiment.id.replace(/^exp-/, "")}-stdout`, "invocation-stdout", stdoutPath),
    artifactRef(fixtureRoot, `artifact-${experiment.id.replace(/^exp-/, "")}-stderr`, "invocation-stderr", stderrPath),
    artifactRef(fixtureRoot, `artifact-${experiment.id.replace(/^exp-/, "")}-result`, "invocation-result", resultPath),
  ];
  const artifactDigests = Object.fromEntries(artifactRefs.map((ref) => [ref.id, ref.digest]));
  const hostVersion = version ?? "missing";
  return {
    schema_version: RAW_SCHEMA,
    payload_version: `${RAW_PAYLOAD_PREFIX}1.0.0`,
    experiment_id: experiment.id,
    host_identity: {
      host: experiment.host,
      surface: experiment.surface,
      version: hostVersion,
      adapter_version: ADAPTER_VERSION,
    },
    surface: experiment.surface,
    tool_name: experiment.expected_tool_name,
    event_source: experiment.expected_event_source,
    started_at: now,
    ended_at: now,
    input: {
      input_kind: experiment.input_kind,
      command: argv,
      probe: "read-only host version probe",
    },
    events: [
      {
        sequence: 1,
        event_name: "started",
        payload_version: "host-event/1.0.0",
        tool_name: experiment.expected_tool_name,
        event_source: experiment.expected_event_source,
        payload: { input_kind: experiment.input_kind },
      },
      {
        sequence: 2,
        event_name: "final",
        final: true,
        payload_version: "host-event/1.0.0",
        tool_name: experiment.expected_tool_name,
        event_source: experiment.expected_event_source,
        payload: { final_status: "unavailable", exit_code: result.status },
      },
    ],
    result: {
      final_status: "unavailable",
      availability_reason: version
        ? `${experiment.host} ${experiment.surface} mechanical version probe returned ${version}, but no host capture payload/event contract was observed`
        : `${experiment.host} ${experiment.surface} mechanical version probe did not produce a parseable version; host capture remains unavailable`,
      permissions: {
        network: "not_used",
        filesystem: "read_only_binary_version_probe",
        environment: "version probe only; no host session launched",
        secrets: "not_requested",
      },
      sandbox: { mode: "not_entered", writable_roots: [] },
      missing_fields: version
        ? ["live_event_stream", "final_result_payload", "artifact_contract"]
        : ["host_version", "live_event_stream", "final_result_payload", "artifact_contract"],
      blind_spots: [
        "installed binary version does not prove host capture payload support",
        "no paid/network host run was performed",
      ],
      artifact_refs: artifactRefs,
      artifact_digests: artifactDigests,
      experiment_plan: [
        `Run ${argv.join(" ")} as a bounded read-only version probe without network or paid model calls.`,
        "Capture a final event stream, permissions, sandbox state, and artifact contract only if the host exposes one.",
        `Keep ${experiment.host} ${experiment.surface} host capture unavailable until replay validates an observed payload contract.`,
      ],
      notes: [
        "mechanical_unavailable_probe records binary availability only and must not be promoted to observed_capture",
      ],
    },
  };
}

function artifactRef(fixtureRoot, id, kind, artifactPath) {
  return {
    id,
    kind,
    root_kind: "fixture_root",
    path: artifactPath,
    digest: sha256File(path.join(fixtureRoot, artifactPath)),
  };
}

function parseVersionProbeStdout(host, stdoutText) {
  const trimmed = stdoutText.trim();
  if (host === "claude") {
    return trimmed.match(/^([0-9]+(?:\.[0-9]+){2}) \(Claude Code\)$/)?.[1] ?? null;
  }
  if (host === "cursor") {
    return trimmed.match(/^([0-9]{4}\.[0-9]{2}\.[0-9]{2}-[0-9a-f]+)$/)?.[1] ?? null;
  }
  if (host === "pi") {
    return trimmed.match(/^([0-9]+(?:\.[0-9]+){2})$/)?.[1] ?? null;
  }
  return null;
}

function unavailableCapture(experiment, now) {
  return {
    schema_version: RAW_SCHEMA,
    payload_version: `${RAW_PAYLOAD_PREFIX}1.0.0`,
    experiment_id: experiment.id,
    host_identity: {
      host: experiment.host,
      surface: experiment.surface,
      version: "missing",
      adapter_version: ADAPTER_VERSION,
    },
    surface: experiment.surface,
    tool_name: experiment.expected_tool_name,
    event_source: experiment.expected_event_source,
    started_at: now,
    ended_at: now,
    input: {
      input_kind: experiment.input_kind,
      probe: "capture-mode placeholder",
      replay_mode: "unprobed-placeholder",
    },
    events: [
      {
        sequence: 1,
        event_name: "final",
        final: true,
        payload_version: "host-event/1.0.0",
        tool_name: experiment.expected_tool_name,
        event_source: experiment.expected_event_source,
        payload: { final_status: "unavailable" },
      },
    ],
    result: {
      final_status: "unavailable",
      availability_reason: "unverified surface remains unavailable until live capture exists",
      permissions: {
        network: "not_probed",
        filesystem: "not_probed",
        environment: "not_probed",
        secrets: "not_requested",
      },
      sandbox: { mode: "not_entered", writable_roots: [] },
      missing_fields: ["host_version", "live_event_stream", "final_result_payload", "artifact_contract"],
      blind_spots: ["capture mode emitted an explicit placeholder, not an observed support claim"],
      artifact_refs: [],
      artifact_digests: {},
      experiment_plan: [
        "Implement a surface-specific controlled probe before treating capture output as observed evidence.",
        "Run this harness in capture mode on a host exposing the surface.",
        "Record raw events and artifacts before enabling any trusted adapter.",
        "Keep capability unavailable until replay validates the observed contract.",
      ],
    },
  };
}

function writeManifestReference(outDir, captures) {
  const manifest = {
    schema_version: "planr.host_capability_manifest_reference.v1",
    manifest_ids: captures
      .map((capture) => `host-${capture.experiment_id.replace(/^exp-/, "")}-manifest`)
      .sort(),
    trusted_adapter_enabled: false,
    source: "capture-mode generated manifest reference",
  };
  writeJson(path.join(outDir, MANIFEST_REF_PATH), manifest);
  return { path: MANIFEST_REF_PATH, digest: sha256File(path.join(outDir, MANIFEST_REF_PATH)) };
}

function writeSchemaReferences(outDir) {
  const sourceRoot = existsSync(path.join(packagedRuntimeRoot(), "v1"))
    ? path.join(packagedRuntimeRoot(), "v1")
    : path.join(packagedFixtureRoot(), "v1");
  const refs = {};
  for (const [key, relativePath] of [
    ["raw", RAW_SCHEMA_REF_PATH],
    ["expected", EXPECTED_SCHEMA_REF_PATH],
    ["provenance", PROVENANCE_SCHEMA_REF_PATH],
  ]) {
    const schema = readJson(path.join(sourceRoot, relativePath));
    writeJson(path.join(outDir, relativePath), schema);
    refs[key] = { path: relativePath, digest: sha256File(path.join(outDir, relativePath)) };
  }
  return refs;
}

function writeProvenanceReference(outDir, captures, schemaRefs) {
  const record = {
    schema_version: "planr.host_capability_provenance.v1",
    schema_ref: schemaRefs.provenance.path,
    schema_digest: schemaRefs.provenance.digest,
    suite_id: "codex-host-capability-phase1",
    captures: captures.map((capture) => {
      const entry = {
        experiment_id: capture.experiment_id,
        source_kind: sourceKindForCapture(capture),
        host: capture.host_identity.host,
        surface: capture.surface,
        input_kind: capture.input.input_kind,
        observation_mode: observationModeForCapture(capture),
        tool_name: capture.tool_name,
        event_source: capture.event_source,
        host_version: capture.host_identity.version,
        adapter_version: capture.host_identity.adapter_version,
        claim_source: claimSourceForCapture(capture),
        availability_reason: capture.result.availability_reason,
        probe_checks: [
          {
            name: "final-event",
            outcome: probeOutcomeForCapture(capture),
            detail: "final event and artifact contract validated by replay",
          },
        ],
        missing_fields: capture.result.missing_fields,
        artifact_ids: capture.result.artifact_refs.map((artifactRef) => artifactRef.id).sort(),
        captured_at: capture.ended_at,
      };
      if (capture[IMPORT_METADATA]?.producer) {
        entry.external_producer = capture[IMPORT_METADATA].producer;
      }
      return entry;
    }),
  };
  writeJson(path.join(outDir, PROVENANCE_REF_PATH), record);
  return { path: PROVENANCE_REF_PATH, digest: sha256File(path.join(outDir, PROVENANCE_REF_PATH)) };
}

function expectedFromCaptures(captures, manifestRef, schemaRefs, provenanceRef, provenanceById = new Map()) {
  const expectedSchemaRefs = Object.fromEntries(
    Object.entries(schemaRefs).map(([key, ref]) => [key, { path: ref.path, digest: ref.digest }]),
  );
  const expectedProvenanceRef = { path: provenanceRef.path, digest: provenanceRef.digest };
  return {
    schema_version: EXPECTED_SCHEMA,
    payload_version: `${EXPECTED_PAYLOAD_PREFIX}1.0.0`,
    suite_id: "codex-host-capability-phase1",
    schema_refs: expectedSchemaRefs,
    provenance_ref: expectedProvenanceRef,
    capability_instances: captures.map((capture) => {
      const provenance = provenanceById.get(capture.experiment_id);
      const claimSource = provenance?.claim_source ?? claimSourceForCapture(capture);
      const checks = provenance?.probe_checks ?? [
        {
          name: "final-event",
          outcome: capture.result.final_status === "available" ? "passed" : "unavailable",
          detail: "final event and artifact contract validated by replay",
        },
      ];
      return {
        raw_capture_id: capture.experiment_id,
        claim_source: claimSource,
        trusted_adapter_enabled: false,
        manifest_ref: manifestRef,
        provenance_ref: expectedProvenanceRef,
        capability_instance: {
          id: `host-${capture.experiment_id}`,
          schema_version: "evidence.contract.v1",
          manifest_id: `host-${capture.experiment_id.replace(/^exp-/, "")}-manifest`,
          manifest_digest: manifestRef.digest,
          host: capture.host_identity.host,
          surface: capture.surface,
          host_version: capture.host_identity.version,
          adapter_version: capture.host_identity.adapter_version,
          environment: environmentBinding(`capture-mode-${os.platform()}`, `env-${capture.experiment_id}`),
          permissions: capture.result.permissions,
          availability: {
            status: capture.result.final_status,
            reason: capture.result.availability_reason,
          },
          probe_result: {
            probe_execution_id: `probe-${capture.experiment_id}`,
            outcome: probeOutcomeForCapture(capture),
            observed_at: capture.ended_at,
            checks,
          },
          observed_payload_contract: {
            schema_ref: RAW_SCHEMA_REF_PATH,
            observation_types: [hostObservationType(capture.host_identity.host, capture.surface)],
          },
          limitations: capture.result.blind_spots,
          captured_at: capture.ended_at,
        },
      };
    }),
  };
}

function environmentBinding(kind, id) {
  return {
    kind,
    id,
    digest: stableDigest({ kind, id }),
  };
}

function hostObservationType(host, surface) {
  return `host.${host.replaceAll("-", "_")}.${surface.replaceAll("-", "_")}`;
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  const getFlag = (name) => {
    const index = args.indexOf(name);
    return index >= 0 ? args[index + 1] : undefined;
  };
  try {
    if (command === "replay") {
      const fixtureRoot = getFlag("--fixture-root");
      if (!fixtureRoot) {
        usage();
        process.exitCode = 2;
        return;
      }
      await replay(fixtureRoot);
      return;
    }
    if (command === "capture") {
      const outDir = getFlag("--out-dir");
      const importFixtureRoot = getFlag("--import-fixture-root");
      if (!outDir) {
        usage();
        process.exitCode = 2;
        return;
      }
      await capture(outDir, { importFixtureRoot });
      return;
    }
    usage();
    process.exitCode = 2;
  } catch (error) {
    console.error(JSON.stringify({ schema_version: SUMMARY_SCHEMA, verdict: "fail", error: error.message }));
    process.exitCode = 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main();
}
