import assert from "node:assert/strict";
import { cp, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), "planr-github-actions-test-"));
const fixtureScripts = path.join(fixtureRoot, "scripts");
const fixtureWorkflows = path.join(fixtureRoot, ".github", "workflows");
const verifier = path.join(fixtureScripts, "verify-github-actions.mjs");
const releaseWorkflow = path.join(fixtureWorkflows, "release.yml");
const ciWorkflow = path.join(fixtureWorkflows, "ci.yml");
const linuxReceiptsWorkflow = path.join(fixtureWorkflows, "linux-receipts.yml");
const securityWorkflow = path.join(fixtureWorkflows, "security.yml");
const fixturePackageJson = path.join(fixtureRoot, "package.json");
const linuxBuildScript = path.join(fixtureScripts, "build-linux-release.sh");
const linuxBuilderDockerfile = path.join(fixtureScripts, "linux-release-builder.Dockerfile");
const linuxVerifyScript = path.join(fixtureScripts, "verify-linux-release-artifact.sh");
const publicLifecycleScript = path.join(fixtureScripts, "verify-public-lifecycle.sh");
const buildReleaseScript = path.join(fixtureScripts, "build-release.sh");
const prepareReleaseScript = path.join(fixtureScripts, "prepare-release-candidate.sh");
const releaseScript = path.join(fixtureScripts, "release.sh");
const localSecurityScript = path.join(fixtureScripts, "security-local.sh");

function runVerifier() {
  return spawnSync(process.execPath, [verifier], {
    cwd: fixtureRoot,
    encoding: "utf8",
  });
}

try {
  await mkdir(fixtureScripts, { recursive: true });
  await cp(path.join(repoRoot, "scripts", "verify-github-actions.mjs"), verifier);
  await cp(path.join(repoRoot, "scripts", "build-linux-release.sh"), linuxBuildScript);
  await cp(path.join(repoRoot, "scripts", "linux-release-builder.Dockerfile"), linuxBuilderDockerfile);
  await cp(path.join(repoRoot, "scripts", "verify-linux-release-artifact.sh"), linuxVerifyScript);
  await cp(path.join(repoRoot, "scripts", "verify-public-lifecycle.sh"), publicLifecycleScript);
  await cp(path.join(repoRoot, "scripts", "build-release.sh"), buildReleaseScript);
  await cp(path.join(repoRoot, "scripts", "prepare-release-candidate.sh"), prepareReleaseScript);
  await cp(path.join(repoRoot, "scripts", "release.sh"), releaseScript);
  await cp(path.join(repoRoot, "scripts", "security-local.sh"), localSecurityScript);
  await cp(path.join(repoRoot, "package.json"), fixturePackageJson);
  await cp(path.join(repoRoot, ".github", "workflows"), fixtureWorkflows, { recursive: true });

  const baseline = runVerifier();
  assert.equal(baseline.status, 0, `baseline workflow fixture must pass:\n${baseline.stderr}`);

  const fixtureFiles = [releaseWorkflow, ciWorkflow, linuxReceiptsWorkflow, linuxBuildScript, linuxBuilderDockerfile, linuxVerifyScript, publicLifecycleScript, buildReleaseScript, prepareReleaseScript, releaseScript, localSecurityScript, fixturePackageJson];
  const baselineSources = new Map(
    await Promise.all(fixtureFiles.map(async (file) => [file, await readFile(file, "utf8")])),
  );
  async function resetFixtures() {
    await Promise.all([...baselineSources].map(([file, source]) => writeFile(file, source)));
  }
  let adversarialCases = 0;
  async function expectRejected(file, mutate, expected, label) {
    await resetFixtures();
    const source = baselineSources.get(file);
    const changed = mutate(source);
    assert.notEqual(changed, source, `${label} mutation must change its fixture`);
    await writeFile(file, changed);
    const result = runVerifier();
    assert.notEqual(result.status, 0, `${label} must fail verification`);
    assert.match(`${result.stdout}\n${result.stderr}`, expected, `${label} failure must explain the invariant`);
    adversarialCases += 1;
  }

  const source = baselineSources.get(releaseWorkflow);
  const smokeMarker = "      - name: Smoke-test binary\n";
  assert.ok(source.includes(smokeMarker), "release fixture must contain the smoke step");

  await expectRejected(
    releaseWorkflow,
    (value) => value.replace(smokeMarker, `${smokeMarker}        if: always()\n`),
    /must be unconditional for every matrix target/u,
    "conditional same-runner smoke",
  );
  await expectRejected(
    releaseWorkflow,
    (value) => value.replace(smokeMarker, `${smokeMarker}        continue-on-error: true\n`),
    /must fail the build job on error/u,
    "non-blocking same-runner smoke",
  );
  await expectRejected(
    releaseWorkflow,
    (value) => value.replace("x86_64-unknown-linux-musl", "x86_64-unknown-linux-gnu"),
    /must build linux-x86_64 natively|must not publish host-glibc Linux targets/u,
    "GNU release target",
  );
  await expectRejected(
    ciWorkflow,
    (value) => value.replace(
      "target: linux-arm64\n            rust_target: aarch64-unknown-linux-musl\n            runner: ubuntu-24.04-arm",
      "target: linux-arm64\n            rust_target: aarch64-unknown-linux-musl\n            runner: ubuntu-24.04",
    ),
    /PR CI must build linux-arm64 natively/u,
    "emulated arm64 PR runner",
  );
  await expectRejected(
    ciWorkflow,
    (value) => `${value}\n# XAI_API_KEY: \${{ secrets.XAI_API_KEY }}\n`,
    /must not contain xAI credentials, Grok installation\/auth, or live model execution/u,
    "xAI credential in PR CI",
  );
  await expectRejected(
    releaseWorkflow,
    (value) => `${value}\n# run: grok --no-auto-update -p \"live release smoke\"\n`,
    /must not contain xAI credentials, Grok installation\/auth, or live model execution/u,
    "live Grok release smoke",
  );
  await expectRejected(
    releaseScript,
    (value) => `${value}\ngrok login\n`,
    /must not contain xAI credentials, Grok installation\/auth, or live model execution/u,
    "Grok login in publication script",
  );
  await expectRejected(
    buildReleaseScript,
    (value) => `${value}\ngrok -p \"live release prompt\" --output-format json\n`,
    /must not contain xAI credentials, Grok installation\/auth, or live model execution/u,
    "Grok model call in release build script",
  );
  await expectRejected(
    prepareReleaseScript,
    (value) => `${value}\ngrok mcp doctor\n`,
    /must not contain xAI credentials, Grok installation\/auth, or live model execution/u,
    "Grok command in candidate preparation script",
  );
  await expectRejected(
    releaseWorkflow,
    (value) => value.replace(
      /      - name: Verify portable Linux artifact\n[\s\S]*?        run: scripts\/verify-linux-release-artifact\.sh\n\n/u,
      "",
    ),
    /must contain an independent Linux compatibility step/u,
    "missing independent compatibility step",
  );
  await expectRejected(
    releaseWorkflow,
    (value) => {
      const verifyBlock = /      - name: Verify portable Linux artifact\n[\s\S]*?        run: scripts\/verify-linux-release-artifact\.sh\n\n/u.exec(value)?.[0];
      assert.ok(verifyBlock, "release fixture must contain compatibility block");
      return value.replace(verifyBlock, "").replace("      - name: Upload release asset\n", `      - name: Upload release asset\n${verifyBlock}`);
    },
    /must complete before release upload/u,
    "compatibility step after upload",
  );
  await expectRejected(
    linuxBuildScript,
    (value) => value.replace(/@sha256:[0-9a-f]{64}/u, ""),
    /build image must use the reviewed immutable/u,
    "unpinned Linux build image",
  );
  await expectRejected(
    linuxBuildScript,
    (value) => value.replace('musl_version="1.2.5-r11"', 'musl_version="latest"'),
    /must pin the reviewed musl package version/u,
    "unpinned musl development prerequisite",
  );
  await expectRejected(
    linuxBuildScript,
    (value) => value.replace("d3b5ab01046a92b9a168b790f516606e320f015cbd4deeb584c5e115a02124ba", "0".repeat(64)),
    /must pin reviewed native APK digest/u,
    "unreviewed native musl-dev digest",
  );
  await expectRejected(
    linuxBuilderDockerfile,
    (value) => value.replace("apk verify /tmp/musl.apk /tmp/musl-dev.apk", "true"),
    /must retain Alpine package signature verification/u,
    "missing Alpine APK signature verification",
  );
  await expectRejected(
    linuxVerifyScript,
    (value) => value.replace(/@sha256:[0-9a-f]{64}/u, ""),
    /runtime image must use the reviewed immutable/u,
    "unpinned compatibility runtime image",
  );
  await expectRejected(
    linuxVerifyScript,
    (value) => value.replace("readelf -l \"$binary\" | grep -q 'INTERP'", "false"),
    /must reject a dynamic program interpreter/u,
    "missing static ELF interpreter check",
  );
  await expectRejected(
    linuxVerifyScript,
    (value) => value.replace("/bin/sh /verify-public-lifecycle.sh", "/bin/sh /not-the-lifecycle.sh"),
    /must execute the fresh public lifecycle/u,
    "missing older-runtime lifecycle",
  );
  await expectRejected(
    linuxVerifyScript,
    (value) => value.replace("cmp \"$binary\" \"$npm_fixture/npm/native/$target/planr\"", "true"),
    /must contain the exact extracted artifact bytes/u,
    "missing npm byte identity check",
  );
  await expectRejected(
    ciWorkflow,
    (value) => value.replace(
      "sha256sum planr-linux-arm64.tar.gz planr-linux-x86_64.tar.gz > SHA256SUMS",
      "sha256sum planr-linux-x86_64.tar.gz > SHA256SUMS",
    ),
    /must aggregate the exact two Linux tarballs/u,
    "incomplete aggregate checksums",
  );
  await expectRejected(
    linuxReceiptsWorkflow,
    (value) => value.replace("runner: ubuntu-24.04-arm", "runner: ubuntu-24.04"),
    /must bind linux-arm64 to ubuntu-24\.04-arm/u,
    "incompatible native receipt host",
  );
  await expectRejected(
    linuxReceiptsWorkflow,
    (value) => value.replace("            .planr/receipts/${{ matrix.target }}.json\n", ""),
    /each native target upload must retain its runner receipt/u,
    "missing native target receipt upload",
  );
  await expectRejected(
    linuxReceiptsWorkflow,
    (value) => value.replace(
      /          node scripts\/verification-runner\.mjs verify-linux-target \\\n            --receipt \.planr\/receipts\/linux-arm64\.json \\\n            --input \.planr\/ci\/selection\.json \\\n            --head "\$GITHUB_SHA"\n/u,
      "",
    ),
    /must replay exactly two target receipts/u,
    "missing aggregate target receipt replay",
  );
  await expectRejected(
    ciWorkflow,
    (value) => `${value}\n# cargo audit --deny warnings\n`,
    /must not run automatic security, secret, or dependency scanners/u,
    "automatic cargo audit",
  );
  await expectRejected(
    ciWorkflow,
    (value) => `${value}\n# trivy fs --scanners vuln,secret .\n`,
    /must not run automatic security, secret, or dependency scanners/u,
    "automatic Trivy scan",
  );
  await expectRejected(
    ciWorkflow,
    (value) => `${value}\n# pnpm docs:verify-shell\n`,
    /retired blanket browser suite/u,
    "automatic blanket browser suite",
  );
  await resetFixtures();
  await writeFile(securityWorkflow, "name: Security\non:\n  pull_request:\njobs:\n  scan:\n    runs-on: ubuntu-latest\n    steps:\n      - run: trivy fs .\n");
  const addedSecurityWorkflow = runVerifier();
  assert.notEqual(addedSecurityWorkflow.status, 0, "a new automatic security workflow must fail verification");
  assert.match(`${addedSecurityWorkflow.stdout}\n${addedSecurityWorkflow.stderr}`, /must not run automatic|must remain absent/u);
  await rm(securityWorkflow);
  adversarialCases += 1;

  console.log(`github_actions_regression=passed adversarial_cases=${adversarialCases} same_runner_smoke_insufficient=true musl_native_pins_lifecycle_checksums_npm_fail_closed=true automatic_scanners_absent=true docs_build_once=true`);
} finally {
  await rm(fixtureRoot, { recursive: true, force: true });
}
