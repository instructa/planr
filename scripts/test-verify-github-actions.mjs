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
const securityWorkflow = path.join(fixtureWorkflows, "security.yml");
const linuxBuildScript = path.join(fixtureScripts, "build-linux-release.sh");
const linuxBuilderDockerfile = path.join(fixtureScripts, "linux-release-builder.Dockerfile");
const linuxVerifyScript = path.join(fixtureScripts, "verify-linux-release-artifact.sh");
const publicLifecycleScript = path.join(fixtureScripts, "verify-public-lifecycle.sh");
const trivyIgnoreFile = path.join(fixtureRoot, ".trivyignore.yaml");

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
  await cp(path.join(repoRoot, ".trivyignore.yaml"), trivyIgnoreFile);
  await cp(path.join(repoRoot, ".github", "workflows"), fixtureWorkflows, { recursive: true });

  const baseline = runVerifier();
  assert.equal(baseline.status, 0, `baseline workflow fixture must pass:\n${baseline.stderr}`);

  const fixtureFiles = [releaseWorkflow, ciWorkflow, securityWorkflow, linuxBuildScript, linuxBuilderDockerfile, linuxVerifyScript, publicLifecycleScript, trivyIgnoreFile];
  const baselineSources = new Map(
    await Promise.all(fixtureFiles.map(async (file) => [file, await readFile(file, "utf8")])),
  );
  async function resetFixtures() {
    await Promise.all([...baselineSources].map(([file, source]) => writeFile(file, source)));
  }
  async function expectRejected(file, mutate, expected, label) {
    await resetFixtures();
    const source = baselineSources.get(file);
    const changed = mutate(source);
    assert.notEqual(changed, source, `${label} mutation must change its fixture`);
    await writeFile(file, changed);
    const result = runVerifier();
    assert.notEqual(result.status, 0, `${label} must fail verification`);
    assert.match(`${result.stdout}\n${result.stderr}`, expected, `${label} failure must explain the invariant`);
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
    securityWorkflow,
    (value) => value.replace("7105f1cd6577f058a9e39d0578f1a99c8a1e481e4d3512cd8a09acfe22a0fdc0", "0".repeat(64)),
    /must pin the reviewed TruffleHog release digest/u,
    "mutated TruffleHog binary digest",
  );
  await expectRejected(
    securityWorkflow,
    (value) => value.replace("8b4376d5d6befe5c24d503f10ff136d9e0c49f9127a4279fd110b727929a5aa9", "0".repeat(64)),
    /must pin the reviewed Trivy release digest/u,
    "mutated Trivy binary digest",
  );
  await expectRejected(
    securityWorkflow,
    (value) => value.replace(" --results=verified --fail --no-update --github-actions", " --results=verified --no-update --github-actions"),
    /must fail closed while scanning verified secrets/u,
    "non-blocking TruffleHog scan",
  );
  await expectRejected(
    securityWorkflow,
    (value) => value.replace(" --results=verified --fail", " --fail"),
    /must fail closed while scanning verified secrets/u,
    "unverified-only TruffleHog contract removed",
  );
  await expectRejected(
    securityWorkflow,
    (value) => value.replace("--scanners secret,misconfig", "--scanners secret"),
    /must scan secrets and misconfigurations/u,
    "Trivy misconfiguration scan removed",
  );
  await expectRejected(
    securityWorkflow,
    (value) => value.replace("      - name: Install pinned security scanners\n", "      - uses: trufflesecurity/trufflehog@6f3c981e7b77f235fd2702dd74af25fc4b72bf11 # v3.96.0\n\n      - name: Install pinned security scanners\n"),
    /uses unreviewed action trufflesecurity\/trufflehog|GitHub-owned-only repository action policy/u,
    "disallowed third-party Security action",
  );
  await expectRejected(
    trivyIgnoreFile,
    (value) => value.replaceAll("scripts/linux-release-builder.Dockerfile", "**"),
    /exceptions must remain limited to the two expiring build-only Dockerfile findings/u,
    "globally broadened Trivy exceptions",
  );
  await expectRejected(
    trivyIgnoreFile,
    (value) => `${value}  - id: AVD-DS-0001\n`,
    /exceptions must remain limited to the two expiring build-only Dockerfile findings/u,
    "additional Trivy exception",
  );
  await expectRejected(
    trivyIgnoreFile,
    (value) => value.replaceAll("    expired_at: 2027-07-26\n", ""),
    /exceptions must remain limited to the two expiring build-only Dockerfile findings/u,
    "missing Trivy exception expiry",
  );

  console.log("github_actions_regression=passed adversarial_cases=24 same_runner_smoke_insufficient=true musl_native_pins_lifecycle_checksums_npm_fail_closed=true security_jobs_fail_closed=true");
} finally {
  await rm(fixtureRoot, { recursive: true, force: true });
}
