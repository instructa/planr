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

Homebrew is the preferred day-to-day package-manager path:

```bash
brew install instructa/tap/planr
```

The tap formula is regenerated automatically on every release.

## npm

Published npm versions bundle platform-native binaries (`darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, `linux-arm64`), so no Rust toolchain is needed and nothing is downloaded at install time:

```bash
npm install -g planr
planr --version
```

Details and publishing flow: [npm Package](NPM.md).

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
planr doctor --client all
planr install codex --dry-run
planr install claude --dry-run
planr install cursor --dry-run
planr prompt mcp
planr prompt cli --client codex
planr prompt http
```

`planr install claude` writes a project `.mcp.json` plus static workflow roles. `planr install cursor` writes `.cursor/mcp.json`, static workflow roles, and Planr skills, then prints a user-level MCP deeplink. `planr install codex` writes only the project MCP snippet. Optional model pins and generated host roles come from a repository-local routing bundle. Dry-runs print exact paths first; no command touches global user configuration.

Skills-and-agents-only setups (no MCP) use `--no-mcp`: `planr install cursor --no-mcp` writes the subagents and skills but no MCP config — the skills drive the `planr` CLI directly, so nothing is lost except MCP tool access.

Runtime surfaces:

```bash
planr mcp                # stdio MCP server for any MCP-capable client
planr serve --port 7526  # localhost HTTP/SSE
```

Open `http://127.0.0.1:7526/review` after `planr serve` for the local browser review workspace.

## Agent Skills And Plugin

The repository ships a plugin under `plugins/planr` for Codex and Claude Code that bundles Planr workflow skills; Claude Code additionally registers static workflow roles. Cursor receives the same skills and static roles through `planr install cursor`. Model-specific role files are optional routing-bundle artifacts. The CLI above must be installed separately. See [Skills](SKILLS.md).

## From Source

Use Cargo when developing Planr or building from a checked-out source tree:

```bash
cargo build --release
PREFIX="$HOME/.local" scripts/install.sh
planr --version
planr doctor --client all
```

The install script copies the selected binary to `PREFIX/bin/planr`. It is idempotent and does not edit global shell or agent-client configuration.

During development, run any command directly without installing: `cargo run -- <command>` (for example `cargo run -- map show`).

## Release Artifact

```bash
scripts/build-release.sh
cat dist/planr-*/SHA256SUMS
cat dist/SHA256SUMS
```

Release builds include a local artifact directory plus a platform tarball named `planr-<os>-<arch>.tar.gz`.
