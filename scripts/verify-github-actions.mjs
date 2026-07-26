import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflowsRoot = path.join(repoRoot, ".github", "workflows");

const expectedActions = new Map([
  ["actions/checkout", { sha: "3d3c42e5aac5ba805825da76410c181273ba90b1", version: "v7.0.1", runtime: "node24" }],
  ["actions/setup-node", { sha: "820762786026740c76f36085b0efc47a31fe5020", version: "v7.0.0", runtime: "node24" }],
  ["actions/upload-artifact", { sha: "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", version: "v7.0.1", runtime: "node24" }],
  ["actions/download-artifact", { sha: "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", version: "v8.0.1", runtime: "node24" }],
  ["aquasecurity/trivy-action", { sha: "ed142fd0673e97e23eac54620cfb913e5ce36c25", version: "v0.36.0", runtime: "composite" }],
  ["trufflesecurity/trufflehog", { sha: "6f3c981e7b77f235fd2702dd74af25fc4b72bf11", version: "v3.96.0", runtime: "composite" }],
]);

const workflowFiles = (await readdir(workflowsRoot))
  .filter((file) => file.endsWith(".yml") || file.endsWith(".yaml"))
  .sort();
const seen = new Map();

for (const file of workflowFiles) {
  const source = await readFile(path.join(workflowsRoot, file), "utf8");
  const lines = source.split("\n");

  for (const [index, line] of lines.entries()) {
    if (!/^\s*uses:/u.test(line)) continue;

    const match = /^\s*uses:\s*([^@\s]+)@([0-9a-f]{40})\s+#\s+(\S+)\s*$/u.exec(line);
    assert.ok(match, `${file}:${index + 1} must use a full immutable commit SHA and exact version comment`);

    const [, action, sha, version] = match;
    const expected = expectedActions.get(action);
    assert.ok(expected, `${file}:${index + 1} uses unreviewed action ${action}`);
    assert.equal(sha, expected.sha, `${file}:${index + 1} has stale SHA for ${action}`);
    assert.equal(version, expected.version, `${file}:${index + 1} has stale version comment for ${action}`);
    seen.set(action, (seen.get(action) ?? 0) + 1);
  }
}

for (const [action, expected] of expectedActions) {
  assert.ok(seen.has(action), `expected workflow action is missing: ${action}`);
  if (expected.runtime === "node24") {
    assert.match(expected.version, /^v(?:7|8)\./u, `${action} must remain on its Node 24 major`);
  }
}

const securityWorkflow = await readFile(path.join(workflowsRoot, "security.yml"), "utf8");
assert.ok(securityWorkflow.includes("version: 3.96.0"), "TruffleHog image version must match the pinned action release");

const releaseWorkflow = await readFile(path.join(workflowsRoot, "release.yml"), "utf8");
const ciWorkflow = await readFile(path.join(workflowsRoot, "ci.yml"), "utf8");
const linuxBuildScript = await readFile(path.join(repoRoot, "scripts", "build-linux-release.sh"), "utf8");
const linuxBuilderDockerfile = await readFile(path.join(repoRoot, "scripts", "linux-release-builder.Dockerfile"), "utf8");
const linuxVerifyScript = await readFile(path.join(repoRoot, "scripts", "verify-linux-release-artifact.sh"), "utf8");
const publicLifecycleScript = await readFile(path.join(repoRoot, "scripts", "verify-public-lifecycle.sh"), "utf8");
for (const target of ["darwin-arm64", "darwin-x86_64", "linux-x86_64", "linux-arm64"]) {
  assert.ok(releaseWorkflow.includes(`target: ${target}`), `release matrix must include ${target}`);
}
for (const [target, rustTarget, runner] of [
  ["linux-x86_64", "x86_64-unknown-linux-musl", "ubuntu-24.04"],
  ["linux-arm64", "aarch64-unknown-linux-musl", "ubuntu-24.04-arm"],
]) {
  const matrixEntry = `- target: ${target}\n            rust_target: ${rustTarget}\n            runner: ${runner}`;
  assert.ok(releaseWorkflow.includes(matrixEntry), `release matrix must build ${target} natively with ${rustTarget}`);
  assert.ok(ciWorkflow.includes(matrixEntry), `PR CI must build ${target} natively with ${rustTarget}`);
}
assert.doesNotMatch(
  releaseWorkflow,
  /(?:x86_64|aarch64)-unknown-linux-gnu/u,
  "release workflow must not publish host-glibc Linux targets",
);

const buildImageDigest = "sha256:3757b14ddcc2057eb91a074dcdd0913bed839b22444bd2229a49eea910ed8736";
const runtimeImageDigest = "sha256:765942a4039992336de8dd5db680586e1a206607dd06170ff0a37267a9e01958";
assert.ok(linuxBuildScript.includes(`rust:1.90.0-alpine3.21@${buildImageDigest}`), "Linux build image must use the reviewed immutable multi-architecture digest");
assert.ok(linuxBuildScript.includes('musl_version="1.2.5-r11"'), "Linux build must pin the reviewed musl package version");
for (const digest of [
  "61e84757a8bfbc0d7fa8f4ce6de9cd4d791714369d78f6a08e5b03510fb2a623",
  "d3b5ab01046a92b9a168b790f516606e320f015cbd4deeb584c5e115a02124ba",
  "721010e6bff908878d9c527428598661be59dde0d9f013f8431d01fd4dd16652",
  "9c4ebdc7e2a29f12de5135cee8f1b92439bfff7c74839b4fb7b422680cf18db4",
]) {
  assert.ok(linuxBuildScript.includes(digest), `Linux build must pin reviewed native APK digest ${digest}`);
}
assert.match(linuxBuildScript, /\| sha256sum -c -/u, "Linux build must verify downloaded APK digests before the image build");
assert.match(linuxBuilderDockerfile, /\| sha256sum -c -/u, "Linux builder image must reverify exact APK bytes before installation");
assert.ok(linuxBuilderDockerfile.includes(`ARG BUILD_IMAGE=rust:1.90.0-alpine3.21@${buildImageDigest}`), "Linux builder Dockerfile must default to the reviewed immutable build image");
assert.match(linuxBuilderDockerfile, /apk verify \/tmp\/musl\.apk \/tmp\/musl-dev\.apk/u, "Linux builder image must retain Alpine package signature verification");
assert.match(linuxBuilderDockerfile, /apk --no-network --repositories-file \/dev\/null add --no-cache/u, "Linux builder image must install only the verified local APKs without repository access");
assert.ok(linuxVerifyScript.includes(`alpine:3.20.8@${runtimeImageDigest}`), "Linux runtime image must use the reviewed immutable older-runtime digest");
assert.match(linuxBuildScript, /uname -m\):\$target:\$cargo_target/u, "Linux build must bind target selection to the native runner architecture");
assert.match(linuxVerifyScript, /uname -m\):\$target:\$cargo_target/u, "Linux verification must bind target selection to the native runner architecture");
assert.match(linuxVerifyScript, /readelf -l "\$binary" \| grep -q 'INTERP'/u, "Linux verification must reject a dynamic program interpreter");
assert.match(linuxVerifyScript, /readelf -d "\$binary" \| grep -q '\(NEEDED\)'/u, "Linux verification must reject shared-library dependencies");
assert.match(linuxVerifyScript, /strings "\$binary" \| grep -Eq 'GLIBC_\[0-9\]'/u, "Linux verification must reject glibc symbol requirements");
assert.match(linuxVerifyScript, /sha256sum -c SHA256SUMS/u, "Linux verification must check embedded artifact checksums");
assert.match(linuxVerifyScript, /--network none/u, "older-runtime lifecycle verification must not use the network");
assert.match(
  linuxVerifyScript,
  /\/bin\/sh \/verify-public-lifecycle\.sh \/artifact\/planr "\$version"/u,
  "Linux verification must execute the fresh public lifecycle",
);
assert.match(linuxVerifyScript, /cmp "\$binary" "\$npm_fixture\/npm\/native\/\$target\/planr"/u, "npm fixture must contain the exact extracted artifact bytes");
assert.match(linuxVerifyScript, /node npm\/bin\/planr\.js --version/u, "Linux verification must execute the npm wrapper over bundled bytes");
for (const command of ["project init", "plan new", "plan split", "map build", "pick", "done", "export"]) {
  assert.ok(publicLifecycleScript.includes(command), `public lifecycle must exercise ${command}`);
}

assert.match(ciWorkflow, /^  linux-portability:\n/mu, "PR CI must contain a Linux portability matrix job");
assert.match(ciWorkflow, /^  linux-portability-checksums:\n/mu, "PR CI must aggregate both Linux tarball checksums");
const portabilityStart = ciWorkflow.indexOf("\n  linux-portability:\n");
const portabilityEnd = ciWorkflow.indexOf("\n  linux-portability-checksums:\n", portabilityStart);
const portabilityJob = ciWorkflow.slice(portabilityStart, portabilityEnd);
assert.doesNotMatch(portabilityJob, /secrets\./u, "PR Linux portability CI must not consume secrets");
assert.match(portabilityJob, /scripts\/build-linux-release\.sh/u, "PR Linux portability CI must use the canonical pinned build");
assert.match(portabilityJob, /scripts\/verify-linux-release-artifact\.sh/u, "PR Linux portability CI must use the canonical compatibility verifier");
assert.match(ciWorkflow, /sha256sum planr-linux-arm64\.tar\.gz planr-linux-x86_64\.tar\.gz > SHA256SUMS/u, "PR CI must aggregate the exact two Linux tarballs");

const smokeStepMarker = "      - name: Smoke-test binary\n";
const smokeStepStart = releaseWorkflow.indexOf(smokeStepMarker);
assert.notEqual(smokeStepStart, -1, "release workflow must contain the binary smoke step");
assert.equal(
  releaseWorkflow.indexOf(smokeStepMarker, smokeStepStart + smokeStepMarker.length),
  -1,
  "release workflow must have exactly one shared matrix smoke step",
);
const nextStepStart = releaseWorkflow.indexOf("\n      - name:", smokeStepStart + smokeStepMarker.length);
assert.notEqual(nextStepStart, -1, "binary smoke step must be followed by another release step");
const smokeStep = releaseWorkflow.slice(smokeStepStart, nextStepStart);
assert.doesNotMatch(smokeStep, /^\s+if:/mu, "binary smoke step must be unconditional for every matrix target");
assert.doesNotMatch(smokeStep, /^\s+continue-on-error:/mu, "binary smoke step must fail the build job on error");
assert.match(
  smokeStep,
  /reported="\$\("\.\/target\/\$PLANR_CARGO_TARGET\/release\/planr" --version\)"/u,
  "release workflow must execute every matrix binary",
);
assert.match(
  smokeStep,
  /expected="planr \$\{TAG#v\}"/u,
  "release workflow must compare runtime output with the release tag",
);

const portabilityStepMarker = "      - name: Verify portable Linux artifact\n";
const portabilityStepStart = releaseWorkflow.indexOf(portabilityStepMarker);
assert.notEqual(portabilityStepStart, -1, "release workflow must contain an independent Linux compatibility step");
const uploadStepStart = releaseWorkflow.indexOf("      - name: Upload release asset\n");
assert.ok(portabilityStepStart < uploadStepStart, "Linux compatibility verification must complete before release upload");
const portabilityStepEnd = releaseWorkflow.indexOf("\n      - name:", portabilityStepStart + portabilityStepMarker.length);
const portabilityStep = releaseWorkflow.slice(portabilityStepStart, portabilityStepEnd);
assert.match(portabilityStep, /startsWith\(matrix\.target, 'linux-'\)/u, "compatibility verification must cover every Linux matrix target");
assert.doesNotMatch(portabilityStep, /continue-on-error:/u, "Linux compatibility failure must block release upload");
assert.match(portabilityStep, /scripts\/verify-linux-release-artifact\.sh/u, "release must use the canonical Linux compatibility verifier");

console.log(
  `github_actions_check=passed workflows=${workflowFiles.length} uses=${[...seen.values()].reduce((sum, count) => sum + count, 0)} node24_actions=4 immutable_sha_pins=true release_arch_runtime_gate=4 linux_static_musl=2 pinned_build_runtime_images=true independent_lifecycle=true`,
);
