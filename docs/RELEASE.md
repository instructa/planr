# Release

Planr V1 releases are built from the Rust binary, public docs, skills, and public assets. The canonical public release source is `https://github.com/instructa/planr`.

The v1 repository-owned public install order is:

1. GitHub Release curl installer.
2. Manual GitHub Release asset download.
3. Homebrew after the tap/formula is published.
4. Cargo/source builds for maintainers and contributors.

The npm wrapper is maintained for development and consumer E2E coverage. Do not present npm or npx as the primary user install path until native binary packages are published with the npm artifact.

## Version Bump

`scripts/release.sh` is the only supported release path. The version lives in one source (`Cargo.toml`) and four distribution manifests must carry it: `Cargo.toml`, `package.json`, `plugins/planr/.codex-plugin/plugin.json`, and `plugins/planr/.claude-plugin/plugin.json`. Manual tagging skips the manifest sync and ships stale plugin versions.

```bash
scripts/release.sh 1.2.0 "one-line release summary"
```

The script enforces, in order:

1. branch is `main`, worktree is clean, `CHANGELOG.md` already has a committed `## [x.y.z]` section, and the tag does not exist;
2. the version is written into all four manifests plus `Cargo.lock`;
3. gates: `cargo test` (includes the manifest drift guard in `tests/e2e.rs`), `npm pack --dry-run`, and `scripts/security-local.sh` (betterleaks + trivy leak gate);
4. one mechanical commit `release x.y.z: <summary>`, tag `vx.y.z`, and a single push of branch plus tag.

Two independent gates back the script:

- `cargo test` fails on every push when any manifest version drifts from `Cargo.toml`.
- The release workflow's `Verify release versions are consistent` step refuses the tag when the tag, any manifest, or the `CHANGELOG.md` section disagree.

## Automated Release Pipeline

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`:

1. `create-release` verifies the tag against `Cargo.toml`, all distribution manifests, and the changelog section, then creates a draft GitHub Release.
2. `build` compiles and packages `planr-<os>-<arch>.tar.gz` for `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, and `linux-arm64`, then uploads each asset to the draft release.
3. `finalize` downloads all uploaded assets, writes one aggregated `SHA256SUMS` covering every tarball, uploads it, and publishes the release.
4. `homebrew-tap` regenerates `Formula/planr.rb` with `scripts/generate-formula.sh` and pushes it to `instructa/homebrew-tap` (installed as `brew install instructa/tap/planr`).

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

The external consumer E2E suite must pass when available on the release machine.

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
- `docs/MCP_CONTRACT.md`
- `docs/fixtures/mcp-contract.json`
- `plugins/`
- `README.md`
- `LICENSE.md`

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
