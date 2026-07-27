import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const workflowsRoot = path.join(repoRoot, ".github", "workflows");
const trivyIgnoreFile = await readFile(path.join(repoRoot, ".trivyignore.yaml"), "utf8");
const localSecurityScript = await readFile(path.join(repoRoot, "scripts", "security-local.sh"), "utf8");

assert.equal(
  trivyIgnoreFile,
  `misconfigurations:
  - id: AVD-DS-0002
    paths:
      - scripts/linux-release-builder.Dockerfile
    statement: This image is an ephemeral build environment that requires root to install verified local APKs and is never shipped or run as a service.
    expired_at: 2027-07-26
  - id: AVD-DS-0026
    paths:
      - scripts/linux-release-builder.Dockerfile
    statement: This image performs one finite build command and exits, so it has no long-running service to health-check.
    expired_at: 2027-07-26
`,
  "Trivy exceptions must remain limited to the two expiring build-only Dockerfile findings",
);
assert.match(localSecurityScript, /--skip-check-update/u, "Local Trivy must use the checks bundled with the reviewed binary");

const expectedActions = new Map([
  ["actions/checkout", { sha: "3d3c42e5aac5ba805825da76410c181273ba90b1", version: "v7.0.1", runtime: "node24" }],
  ["actions/setup-node", { sha: "820762786026740c76f36085b0efc47a31fe5020", version: "v7.0.0", runtime: "node24" }],
  ["actions/upload-artifact", { sha: "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", version: "v7.0.1", runtime: "node24" }],
  ["actions/download-artifact", { sha: "3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", version: "v8.0.1", runtime: "node24" }],
]);

const workflowFiles = (await readdir(workflowsRoot))
  .filter((file) => file.endsWith(".yml") || file.endsWith(".yaml"))
  .sort();
const seen = new Map();
const workflowSources = new Map();
const releaseScriptFiles = [
  "build-release.sh",
  "build-linux-release.sh",
  "prepare-release-candidate.sh",
  "release.sh",
  "verify-linux-release-artifact.sh",
  "verify-public-lifecycle.sh",
];

for (const file of workflowFiles) {
  const source = await readFile(path.join(workflowsRoot, file), "utf8");
  workflowSources.set(file, source);
  const lines = source.split("\n");

  for (const [index, line] of lines.entries()) {
    if (!/^\s*(?:-\s*)?uses:/u.test(line)) continue;

    const match = /^\s*(?:-\s*)?uses:\s*([^@\s]+)@([0-9a-f]{40})\s+#\s+(\S+)\s*$/u.exec(line);
    assert.ok(match, `${file}:${index + 1} must use a full immutable commit SHA and exact version comment`);

    const [, action, sha, version] = match;
    const expected = expectedActions.get(action);
    assert.ok(expected, `${file}:${index + 1} uses unreviewed action ${action}`);
    assert.equal(sha, expected.sha, `${file}:${index + 1} has stale SHA for ${action}`);
    assert.equal(version, expected.version, `${file}:${index + 1} has stale version comment for ${action}`);
    seen.set(action, (seen.get(action) ?? 0) + 1);
  }
}

const releaseBoundarySources = new Map(
  await Promise.all(
    releaseScriptFiles.map(async (file) => [
      `scripts/${file}`,
      await readFile(path.join(repoRoot, "scripts", file), "utf8"),
    ]),
  ),
);
for (const [file, source] of [...workflowSources, ...releaseBoundarySources]) {
  assert.doesNotMatch(
    source,
    /(?:xai|x\.ai|\bgrok\b)/iu,
    `${file} must not contain xAI credentials, Grok installation/auth, or live model execution; Grok verification is maintainer-local only`,
  );
}

for (const [action, expected] of expectedActions) {
  assert.ok(seen.has(action), `expected workflow action is missing: ${action}`);
  if (expected.runtime === "node24") {
    assert.match(expected.version, /^v(?:7|8)\./u, `${action} must remain on its Node 24 major`);
  }
}

const securityWorkflow = await readFile(path.join(workflowsRoot, "security.yml"), "utf8");
assert.doesNotMatch(
  securityWorkflow,
  /^\s*(?:-\s*)?uses:\s*(?:aquasecurity|trufflesecurity)\//mu,
  "Security workflow must respect the GitHub-owned-only repository action policy",
);
for (const [expected, label] of [
  ["https://github.com/trufflesecurity/trufflehog/releases/download/v3.96.0/trufflehog_3.96.0_linux_amd64.tar.gz", "TruffleHog release URL"],
  ["7105f1cd6577f058a9e39d0578f1a99c8a1e481e4d3512cd8a09acfe22a0fdc0", "TruffleHog release digest"],
  ["https://github.com/aquasecurity/trivy/releases/download/v0.70.0/trivy_0.70.0_Linux-64bit.tar.gz", "Trivy release URL"],
  ["8b4376d5d6befe5c24d503f10ff136d9e0c49f9127a4279fd110b727929a5aa9", "Trivy release digest"],
  ["https://github.com/zizmorcore/zizmor/releases/download/v1.24.1/zizmor-x86_64-unknown-linux-gnu.tar.gz", "zizmor release URL"],
  ["a8000f3c683319a523d3b20df0e75457ba591f049cfcbfa98966631b56733c03", "zizmor release digest"],
]) {
  assert.ok(securityWorkflow.includes(expected), `Security workflow must pin the reviewed ${label}`);
}
assert.match(securityWorkflow, /sha256sum --check -/u, "Security scanner downloads must be checksum verified");
assert.match(
  securityWorkflow,
  /trufflehog git "file:\/\/\$GITHUB_WORKSPACE" --results=verified --fail --no-update --github-actions/u,
  "TruffleHog must fail closed while scanning verified secrets across Git history",
);
assert.match(securityWorkflow, /--scanners secret,misconfig/u, "Trivy must scan secrets and misconfigurations");
assert.match(securityWorkflow, /--ignorefile \.trivyignore\.yaml/u, "Trivy CI must use the reviewed narrow ignore file");
assert.match(securityWorkflow, /--skip-check-update/u, "Trivy must use the checks bundled with the reviewed binary");
assert.match(securityWorkflow, /--exit-code 1/u, "Trivy findings must fail the Security job");
assert.doesNotMatch(
  securityWorkflow,
  /(?:python3\s+-m\s+pip\s+install[^\n]*\buv\b|\buvx\b)/u,
  "Security workflow must not install mutable uv or zizmor inputs",
);
assert.match(securityWorkflow, /\n\s*zizmor \. \\\n/u, "GitHub Actions Security must run the checksum-verified zizmor binary");

const releaseWorkflow = await readFile(path.join(workflowsRoot, "release.yml"), "utf8");
const ciWorkflow = await readFile(path.join(workflowsRoot, "ci.yml"), "utf8");
const linuxReceiptsWorkflow = await readFile(path.join(workflowsRoot, "linux-receipts.yml"), "utf8");
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
assert.match(ciWorkflow, /^  router:\n/mu, "PR CI must contain an always-running verification router");
assert.match(ciWorkflow, /^  workflow_dispatch:$/mu, "CI must support an explicit native-Linux-only dispatch on a reviewed branch SHA");
const routerStart = ciWorkflow.indexOf("\n  router:\n");
const routerEnd = ciWorkflow.indexOf("\n  docs:\n", routerStart);
const routerJob = ciWorkflow.slice(routerStart, routerEnd);
assert.doesNotMatch(routerJob, /^\s+if:/mu, "verification router must always run");
for (const output of ["profile", "policy_version", "policy_digest", "changed_files_digest", "docs", "quality", "release", "linux_portability"]) {
  assert.match(routerJob, new RegExp(`^      ${output}: \\$\\{\\{ steps\\.route\\.outputs\\.${output} \\}\\}$`, "mu"), `verification router must export ${output}`);
}
assert.match(routerJob, /node scripts\/ci-router\.mjs route/u, "verification router must use the repository-owned deterministic helper");
assert.match(routerJob, /name: verification-selection/u, "verification routing evidence must cross jobs only as an explicit artifact");
assert.match(routerJob, /docs=false\\nquality=false\\nrelease=false\\nlinux_portability=true/u, "manual CI dispatch must select only native Linux evidence");
for (const [jobHeader, condition] of [
  ["  docs:\n    name: Documentation\n", "if: needs.router.outputs.docs == 'true'"],
  ["  quality:\n    name: Quality Gates\n", "if: needs.router.outputs.quality == 'true'"],
  ["  release-contracts:\n    name: Release and Security Gates\n", "if: needs.router.outputs.release == 'true'"],
  ["  linux-portability:\n    name: Portable Linux ${{ matrix.target }}\n", "if: needs.router.outputs.linux_portability == 'true'"],
]) {
  const start = ciWorkflow.indexOf(jobHeader);
  assert.notEqual(start, -1, `PR CI must contain ${jobHeader.trim()}`);
  assert.ok(ciWorkflow.slice(start, start + 240).includes(condition), `${jobHeader.trim()} must use its router output`);
}
assert.doesNotMatch(ciWorkflow, /(?:secrets\.|actions\/cache@|\bcache:)\b/u, "PR CI dependency acceleration must not consume secrets or masquerade as evidence");
const summaryStart = ciWorkflow.indexOf("\n  summary:\n");
assert.notEqual(summaryStart, -1, "PR CI must contain one stable summary job");
const summaryJob = ciWorkflow.slice(summaryStart);
assert.match(summaryJob, /name: CI Summary/u, "PR CI summary check name must remain stable");
assert.match(summaryJob, /if: always\(\)/u, "PR CI summary must run after failures and skips");
assert.match(summaryJob, /node scripts\/ci-router\.mjs summary/u, "PR CI summary must use the fail-closed result verifier");
for (const result of ["needs.docs.result", "needs.quality.result", "needs.release-contracts.result", "needs.linux-portability-checksums.result"]) {
  assert.ok(summaryJob.includes(result), `PR CI summary must inspect ${result}`);
}
const portabilityStart = ciWorkflow.indexOf("\n  linux-portability:\n");
const portabilityEnd = ciWorkflow.indexOf("\n  linux-portability-checksums:\n", portabilityStart);
const portabilityJob = ciWorkflow.slice(portabilityStart, portabilityEnd);
assert.doesNotMatch(portabilityJob, /secrets\./u, "PR Linux portability CI must not consume secrets");
assert.equal((portabilityJob.match(/run-linux-target/g) ?? []).length, 1, "the native matrix must invoke each target runner exactly once");
assert.match(portabilityJob, /^            \.planr\/receipts\/\$\{\{ matrix\.target \}\}\.json$/mu, "PR Linux portability CI must upload each native target receipt");
const portabilityAggregate = ciWorkflow.slice(portabilityEnd, summaryStart);
assert.equal((portabilityAggregate.match(/verify-linux-target/g) ?? []).length, 2, "PR CI aggregate must replay exactly two native target receipts");
assert.match(portabilityAggregate, /name: native-linux-receipts-\$\{\{ github\.sha \}\}/u, "PR CI must retain exact-SHA native receipt evidence");
assert.match(ciWorkflow, /sha256sum planr-linux-arm64\.tar\.gz planr-linux-x86_64\.tar\.gz > SHA256SUMS/u, "PR CI must aggregate the exact two Linux tarballs");
assert.equal((summaryJob.match(/if: github\.event_name != 'workflow_dispatch'/g) ?? []).length, 2, "manual native-only runs must not emit a promotion receipt");

assert.match(linuxReceiptsWorkflow, /^name: Native Linux receipts$/mu, "native Linux evidence must have one stable workflow identity");
assert.match(linuxReceiptsWorkflow, /^  workflow_dispatch:$/mu, "native Linux evidence must be explicitly dispatched for a reviewed SHA");
assert.doesNotMatch(linuxReceiptsWorkflow, /(?:secrets\.|pull_request_target|push:)/u, "native Linux evidence must not consume secrets or run implicitly");
const nativeTargetStart = linuxReceiptsWorkflow.indexOf("\n  target:\n");
const nativeAggregateStart = linuxReceiptsWorkflow.indexOf("\n  aggregate:\n", nativeTargetStart);
assert.ok(nativeTargetStart >= 0 && nativeAggregateStart > nativeTargetStart, "native Linux evidence must separate target and aggregate jobs");
const nativeTargetJob = linuxReceiptsWorkflow.slice(nativeTargetStart, nativeAggregateStart);
assert.match(nativeTargetJob, /^            \.planr\/receipts\/\$\{\{ matrix\.target \}\}\.json$/mu, "each native target upload must retain its runner receipt");
for (const [target, runner] of [
  ["linux-x86_64", "ubuntu-24.04"],
  ["linux-arm64", "ubuntu-24.04-arm"],
]) {
  const matrixEntry = `- target: ${target}\n            runner: ${runner}`;
  assert.ok(linuxReceiptsWorkflow.includes(matrixEntry), `native receipt workflow must bind ${target} to ${runner}`);
  assert.ok(linuxReceiptsWorkflow.includes(`.planr/receipts/${target}.json`), `native aggregate must consume the ${target} receipt`);
  assert.ok(linuxReceiptsWorkflow.includes(`dist/planr-${target}.tar.gz`), `native receipt workflow must retain the ${target} archive`);
}
assert.equal((linuxReceiptsWorkflow.match(/run-linux-target/g) ?? []).length, 1, "the matrix must invoke each native target exactly once");
assert.equal((linuxReceiptsWorkflow.match(/verify-linux-target/g) ?? []).length, 2, "the aggregate must replay exactly two target receipts");
assert.match(linuxReceiptsWorkflow, /name: native-linux-receipts-\$\{\{ github\.sha \}\}/u, "aggregate native evidence must be named by exact SHA");
assert.match(linuxReceiptsWorkflow, /sha256sum planr-linux-arm64\.tar\.gz planr-linux-x86_64\.tar\.gz > SHA256SUMS/u, "native evidence must aggregate the exact two archives");

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
