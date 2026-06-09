# Install Planr

## Recommended User Install

Planr's canonical release source is:

- https://github.com/instructa/planr/releases

Install the current GitHub Release with the repo-owned installer:

```bash
curl -fsSL https://raw.githubusercontent.com/instructa/planr/main/scripts/install.sh | sh
planr --version
planr doctor --client all
```

The installer downloads `planr-<os>-<arch>.tar.gz` from the latest GitHub Release. Override the release source with:

```bash
PLANR_DOWNLOAD=1 PLANR_VERSION=v1.0.0 sh scripts/install.sh
PLANR_DOWNLOAD=1 PLANR_REPO=your-org/planr PLANR_VERSION=v1.0.0 sh scripts/install.sh
PLANR_DOWNLOAD=1 PLANR_TARGET=darwin-arm64 sh scripts/install.sh
PLANR_DOWNLOAD=1 PLANR_RELEASE_BASE_URL=https://example.com/releases sh scripts/install.sh
```

Download installs verify `SHA256SUMS` from the same release location by default. Use `PLANR_SKIP_CHECKSUM=1` only for local development mirrors where the checksum file is intentionally unavailable.

## Homebrew Tap

Homebrew is the preferred day-to-day package-manager path after the tap is published. The expected command is `brew install instructa/tap/planr`.

Until the tap is published, use the GitHub Release installer or manual release asset download.

## Manual GitHub Release Install

Download the matching asset from GitHub Releases:

```bash
tar -xzf planr-darwin-arm64.tar.gz
PREFIX="$HOME/.local" PLANR_BIN="$PWD/planr" scripts/install.sh
planr --version
```

Windows native release assets are not part of the current public install contract. Windows users should use WSL with the Linux release asset or build from source until a Windows asset is published.

## Client Setup

Planr does not edit global agent configuration during install. From a project, use:

```bash
planr install codex --dry-run
planr install claude --dry-run
planr install cursor --dry-run
planr prompt mcp
planr prompt cli --client codex
planr prompt http
```

## From Source

Use Cargo when developing Planr or building from a checked-out source tree:

```bash
cargo build --release
PREFIX="$HOME/.local" scripts/install.sh
planr --version
planr doctor --client all
```

The install script copies the selected binary to `PREFIX/bin/planr`. It is idempotent and does not edit global shell or agent-client configuration.

## Release Artifact

```bash
scripts/build-release.sh
cat dist/planr-*/SHA256SUMS
cat dist/SHA256SUMS
```

Release builds include a local artifact directory plus a platform tarball named `planr-<os>-<arch>.tar.gz`.
