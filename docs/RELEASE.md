# Release

Planr V1 releases are built from the Rust binary, public docs, skills, and public assets. The canonical public release source is `https://github.com/instructa/planr`.

The v1 repository-owned public install order is:

1. GitHub Release curl installer.
2. Manual GitHub Release asset download.
3. Homebrew after the tap/formula is published.
4. npm (`npm install -g planr`) with bundled platform binaries.
5. Cargo/source builds for maintainers and contributors.

Published npm versions bundle platform-native binaries; see <https://planr.so/docs/operations/release>.

## Candidate and publication

`scripts/prepare-release-candidate.sh` is the only supported version-transition
path. Run it from clean, reviewed source on a release branch. It synchronizes
`Cargo.toml`, `Cargo.lock`, `package.json`, both plugin manifests under
`plugins/planr/`, `.cursor-plugin/plugin.json`, and the generated CLI/MCP
references. It never stages, commits, tags, pushes, or publishes. Commit those
changes with the changelog and any release-contract transition, then run the
full candidate verification and independent review on that exact commit.

`scripts/release.sh` is the only supported publication path. It runs on clean
`main`, requires every version and generated reference to already match the
requested version, verifies an independently green CI run and human approval
for the exact `HEAD` SHA, and only then creates and pushes the annotated tag.
It does not replay the Rust, docs, or packaging suites already proven by that
CI run. Security, secret, dependency, and workflow scanners are deliberate
local maintainer preflight commands rather than automatic pull-request or push
CI evidence. Editing manifests by hand or publishing an unprepared commit skips
this ownership boundary.

```bash
scripts/prepare-release-candidate.sh 1.2.0
```

```bash
export PLANR_RELEASE_CI_RECEIPT=/path/to/downloaded/promotion-receipt.json
export PLANR_RELEASE_APPROVAL=/path/to/exact-sha-release-approval.json
scripts/release.sh 1.2.0 "one-line release summary"
```

Download `release-promotion-<sha>` from the successful `CI` run for the exact
main commit. The approval file uses schema `planr.release-approval.v1` and
contains only `approval_id`, `source_sha`, `version`, `decision: "approved"`,
`approved_by`, and `approved_at` in addition to `schema_version`. Publication
queries the recorded GitHub Actions run and rejects a stale SHA, non-main or
non-push run, failed conclusion, repository mismatch, or non-approved decision.

External evaluation is conditional. When the evaluated workflow subject or its
explicit evaluation policy changed since the previous release tag, also set:

```bash
export PLANR_RELEASE_EVAL_SUITE="$HOME/projects/planr-evals/suites/planr-lean-skills-dogfood.suite.json"
export PLANR_RELEASE_EVAL_RECEIPT=/path/to/sanitized-release-eval-receipt.json
export PLANR_RELEASE_EVAL_DB=/path/to/planr-evals/results/eval.sqlite
export PLANR_RELEASE_PLANR_BIN=/path/to/reviewed/candidate/planr
```

Maintainer benchmarks, baselines, model/effort runs, and results live outside
the public repository in `~/projects/planr-evals`; that workspace and its exact
layout are not a Planr runtime contract. All external evaluation paths above are explicit so a
release cannot silently use the product repository's ordinary `.planr` database
or a stale bundled suite. The receipt is a short-lived local pointer containing
only `schema_version`, comparison and candidate-run identities, suite and
candidate-revision digests, and creation/expiry timestamps. Prompts,
completions, credentials, personal paths, raw runs, reports, databases, and the
receipt itself are never committed to Planr.

Fixture paths inside the private suite resolve relative to the suite file's
directory. The external eval workspace must therefore contain its complete
fixture tree; release verification does not read ignored files from the Planr
checkout.

Generate the candidate revision with
`node scripts/verify-release-eval-receipt.mjs --print-candidate-revision`. The
revision hashes every tracked or non-ignored release-source file, including its
normalized executable mode, so changing any candidate source invalidates the
receipt. The separate evaluated-subject revision binds the stored model run to
the five workflow files the benchmark evaluates; changing either the exact
release source or that evaluated subset invalidates the receipt. At release time the candidate binary canonicalizes the explicitly supplied suite,
requires a Planr-validated effective route observation (host report, telemetry
receipt, process exit, or local observation), recomputes the comparison from its
stored baseline/candidate/policy identities, and gates that fresh result.
Requested-only values and policy or binding metadata are never
effective-treatment proof.

The two scripts enforce, in order:

1. candidate preparation starts from a clean worktree and a nonexistent tag;
2. the candidate version is written into all synchronized manifests;
3. frozen workspace synchronization cannot change `pnpm-lock.yaml`;
4. the candidate build synchronizes `Cargo.lock`, then regenerates and strictly checks both references without Git mutation;
5. candidate source, changelog, contracts, and generated files are committed and independently reviewed before publication approval;
6. publication requires clean `main`, the exact prepared versions/references, a committed changelog section, and no existing tag;
7. publication validates the exact-SHA CI and approval receipts; when the evaluated subject or policy changed, the reviewed candidate binary also validates the sanitized eval receipt and recomputed comparison;
8. publication creates and pushes only the annotated `vx.y.z` tag for that reviewed commit.

Two independent gates back the script:

- `cargo test` fails on every push when any manifest version drifts from `Cargo.toml`.
- The release workflow's `Verify release versions are consistent` step refuses the tag when the tag, any manifest, or the `CHANGELOG.md` section disagree.

Public CI runs `npm run verify:release-eval-gate` with a synthetic suite, fake
binary, and temporary database. It verifies the fail-closed mechanism without a
provider call, private benchmark, API key, or release receipt. Model-backed
quality decisions remain a local maintainer preflight.

## Alpha Channel (Pre-Releases)

Pre-release versions use the same script and pipeline with a semver pre-release suffix:

```bash
scripts/release.sh 1.2.0-alpha.1 "one-line summary"
```

The changelog section requirement applies verbatim (`## [1.2.0-alpha.1]`). What changes downstream:

- The GitHub Release is marked as a **prerelease**, so `releases/latest` — and with it the curl installer's default — stays on the last stable version. Testers pin explicitly: `PLANR_VERSION=1.2.0-alpha.1 sh install.sh`.
- npm publishes under the **`alpha` dist-tag** instead of `latest`: plain `npm install -g planr` keeps resolving stable, testers opt in with `npm install -g planr@alpha`.
- The **Homebrew tap never moves** on pre-release tags.

Only `-alpha.N`, `-beta.N`, and `-rc.N` suffixes are accepted; everything else the script rejects.

## Automated Release Pipeline

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`:

<!-- planr:linux-release-portability:start surface=maintainerRelease schema=1 -->
> **Linux release portability — corrected**
>
> Contract state: `status=corrected`; `affectedThrough=v1.7.2`; `correctedFrom=v1.7.3`.
> Published Linux release, installer, and npm binaries through v1.7.2 require GLIBC_2.39; macOS is unaffected.
> Starting with v1.7.3, current Linux release, installer, and npm artifacts are static-musl executables and do not require glibc.
> On an affected Linux release, build from source on the target distribution or upgrade to v1.7.3.
<!-- planr:linux-release-portability:end surface=maintainerRelease schema=1 -->

When this contract changes, update `docs/contracts/LINUX_RELEASE_PORTABILITY.json`, run `pnpm docs:sync-linux-portability`, and commit every synchronized notice before running the release gates.

1. `create-release` verifies the tag against `Cargo.toml`, all distribution manifests, and the changelog section, then creates a draft GitHub Release.
2. `build` compiles and packages `planr-<os>-<arch>.tar.gz` for `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, and `linux-arm64`, then uploads each asset to the draft release. Future Linux candidates use native x86_64/arm64 GitHub runners and the same digest-pinned Rust 1.90.0 Alpine/musl image. Before upload, the extracted tarball must pass embedded checksums, static ELF checks (no interpreter, shared-library dependency, or glibc symbol), a fresh project/plan/map/pick/done/export lifecycle in digest-pinned Alpine 3.20.8 with networking disabled, and exact-byte npm wrapper execution.
3. `finalize` downloads all uploaded assets, writes one aggregated `SHA256SUMS` covering every tarball, uploads it, and publishes the release.
4. `npm-publish` downloads the release assets, verifies them against `SHA256SUMS`, bundles the four platform binaries into `npm/native/`, smoke-tests the wrapper, and publishes to npm via Trusted Publishing (OIDC). Runs only when the repository variable `NPM_PUBLISH_ENABLED` is `true`; requires the one-time Trusted Publisher setup described at <https://planr.so/docs/operations/release>.
5. `homebrew-tap` regenerates `Formula/planr.rb` with `scripts/generate-formula.sh` and pushes it to `instructa/homebrew-tap` (installed as `brew install instructa/tap/planr`).

## Changelog

`CHANGELOG.md` at the repository root is the persistent release log ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format). Maintain it continuously, not at release time only:

- Every user-visible change (CLI surface, JSON envelope, skills, MCP/HTTP contract, install paths) lands in the `[Unreleased]` section in the same PR or commit that makes the change.
- Before pushing a release tag, rename `[Unreleased]` to the new version with the release date, add a fresh empty `[Unreleased]` section, and update the compare links at the bottom. The tag must not ship with a non-empty `[Unreleased]` section describing its own changes.
- The version section is the source for the GitHub Release notes body.

The Homebrew job only runs when the repository variable `HOMEBREW_TAP_ENABLED` is `true` and requires a `TAP_GITHUB_TOKEN` secret with write access to `instructa/homebrew-tap`. The tap repository must exist before enabling it.

## Preflight

Run:

```bash
scripts/ci-local.sh
scripts/security-local.sh
```

`scripts/security-local.sh`, `cargo audit --deny warnings`, and local
`zizmor .` are on-demand maintainer checks. Pull-request and push workflows do
not install or execute BetterLeaks, Trivy, TruffleHog, cargo-audit, zizmor, or
equivalent dependency/security scanners.

The external consumer E2E suite must pass when available on the release machine.
Pull-request CI separately builds both Linux architectures through the canonical
containerized release script, runs the full portability contract without
secrets or publication permissions, and aggregates checksums for the exact two
candidate tarballs. A same-runner `--version` smoke is useful architecture
evidence but is not Linux compatibility proof by itself.

## Build Artifact

Create the local release artifact:

```bash
scripts/build-release.sh
cat dist/planr-*/SHA256SUMS
```

The artifact contains:

- `planr`
- `README.md`
- `LICENSE.md`
- `SHA256SUMS`

The GitHub Release upload asset is:

- `dist/planr-<os>-<arch>.tar.gz`

The tarball checksum is written to `dist/SHA256SUMS`.

The release installer downloads and verifies `SHA256SUMS` from the same release URL unless `PLANR_SKIP_CHECKSUM=1` is set for a development mirror.

## npm Dry-Run

Verify npm package contents as a development-package check:

```bash
npm pack --dry-run
```

The package must include:

- `npm/bin/planr.js`
- `docs/`
- `docs/fixtures/mcp-contract.json`
- `plugins/`
- `README.md`
- `LICENSE.md`

`npm/native/` platform binaries exist only in the `npm-publish` CI job; the local dry-run does not include them.

## Install Smoke

After building:

```bash
node npm/bin/planr.js --version
PREFIX="$(mktemp -d)" scripts/install.sh
PLANR_BIN="$(find dist -path '*/planr' -type f | head -n 1)" PREFIX="$(mktemp -d)" scripts/install.sh
```

Then run:

```bash
PLANR_BIN=planr npm run test:npm-planr
```

from the external consumer E2E project.

## Release Notes Checklist

Before publishing, record:

- `CHANGELOG.md` updated: `[Unreleased]` rolled into the tagged version section;
- exact commit or source snapshot;
- `cargo test` result;
- consumer E2E result;
- `npm pack --dry-run` file list;
- release artifact checksum;
- GitHub Release asset name and checksum;
- security/leak scan result;
- known risks or intentionally unsupported platforms.
