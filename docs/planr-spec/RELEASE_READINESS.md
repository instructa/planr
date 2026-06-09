# Release Readiness

## Release Channels

- Source build from GitHub.
- Prebuilt binaries for macOS arm64/x86_64 and Linux arm64/x86_64.
- Homebrew formula after initial stable release.
- npm wrapper optional only if Node-based install convenience is needed.

## Versioning

- Semantic versioning.
- Database schema version stored in SQLite.
- MCP contract version exposed by `planr mcp --version` or initialize metadata.

## Packaging Requirements

- REQ-REL-001: Release binaries must be checksummed.
- REQ-REL-002: Install script must be readable, idempotent, and avoid hidden global config edits.
- REQ-REL-003: Agent integration commands must support dry-run.
- REQ-REL-004: Upgrade must not rewrite `.planr` files without an explicit Planr write command.
- REQ-REL-005: Download installs must verify `SHA256SUMS` from the same release location by default.
- REQ-REL-006: `PLANR_SKIP_CHECKSUM=1` may exist only as an explicit development-mirror escape hatch.
- REQ-REL-007: Release docs must distinguish release installs, Homebrew after tap publication, source builds, and Windows/WSL expectations.

## Upgrade Readiness

- Open current Planr workspaces without rewriting Markdown artifacts.
- Back up the SQLite database before schema upgrades.
- Report package import/export results before mutation.
- Provide rollback instructions for binary and database backups.

## Documentation Readiness

Required:

- README.
- Install guide.
- CLI reference generated from actual help.
- MCP integration guide.
- Codex guide.
- Claude Code guide.
- Cursor guide.
- Package import/export guide.
- Security and privacy notes.
- Troubleshooting/doctor guide.

## Security Review

Before public release:

- Review install script.
- Review MCP mutation tools.
- Review HTTP bind/auth behavior.
- Review log scrubbing.
- Review secret detection.
- Review dependency supply chain.

## QA Release Checklist

- `planr project init` smoke test in empty repo.
- Package import/export smoke test.
- Codex MCP registration smoke test.
- Claude Code MCP config smoke test.
- Cursor MCP config smoke test.
- Concurrent pick test.
- Review/fix loop test.
- Recovery sweep test.
- Local browser review workspace smoke test.
- Git/PR review evidence test.
- Export/import roundtrip test.
- Template package import preview and confirm test.
- `scripts/build-release.sh`, checksum verification, installer file-url smoke test, and `npm pack --dry-run`.

## Rollback

- Binary rollback: install previous version.
- Database rollback: restore the automatic backup created before schema upgrades.
- Plan files: never silently rewritten.

## Legal/Platform Items

- Confirm license review for any retained code, docs, or assets.
- Ensure final product docs use Planr-owned naming, examples, and command vocabulary.
- Ensure README states local privacy behavior clearly.

## Launch Criteria

- All regression reviews pass.
- Docs cover first-run setup for Codex, Claude Code, and Cursor.
- `planr doctor --client all` produces actionable output.
- `planr prompt cli|mcp|http` prints actionable instructions without editing config.
- Fresh consumer E2E passes in `~/projects/planr-test`.
- No content telemetry.
- Release smoke tests do not rewrite workspace files unexpectedly.
