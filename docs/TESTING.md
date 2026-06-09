# Testing

Planr has two test layers plus a release-grade V1.1 verification ladder.

## In-Repo Tests

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Focused V1.1 checks should be run when their surfaces change:

```bash
cargo test recovery_sweep -- --nocapture
cargo test local_review_workspace -- --nocapture
cargo test review_evidence -- --nocapture
cargo test template_export_import -- --nocapture
```

## Consumer E2E Project

The standalone consumer suite lives at:

```bash
~/projects/planr-test
```

Run against the local native binary:

```bash
cd ~/projects/planr
cargo build --release
cd ~/projects/planr-test
npm test
```

Run through the npm package wrapper:

```bash
cd ~/projects/planr
npm link
cd ~/projects/planr-test
npm link planr
npm run test:npm-planr
```

The consumer suite exercises every public command group and subcommand, MCP stdio, local HTTP/SSE, import/export, review gates, install helpers, and generated plan files.

## V1.1 Release Verification

Before calling V1.1 complete, run the full in-repo and consumer ladder:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
scripts/build-release.sh
(cd dist && shasum -a 256 -c SHA256SUMS)
(cd dist/planr-1.0.0 && shasum -a 256 -c SHA256SUMS)
npm pack --dry-run
```

Installer changes also require:

```bash
/Users/kregenrek/.agents/skills/shellck/scripts/run_shellck.sh scripts/install.sh
PREFIX="$(mktemp -d)" PLANR_DOWNLOAD=1 PLANR_RELEASE_BASE_URL="file://$PWD/dist" scripts/install.sh
```

Live behavior must be proved with:

- a fresh consumer project at `~/projects/planr-test`;
- MCP stdio contract checks against `docs/fixtures/mcp-contract.json`;
- localhost HTTP/SSE checks and the browser review workspace at `/review`;
- recovery sweep checks for stale, timed-out, and retryable work;
- Git/PR review evidence checks that do not inline source content;
- package export/import checks for templates, logs, plan files, and review artifacts;
- prompt output checks for CLI, MCP, and HTTP setup text;
- a forbidden-reference scrub across public repo files.

## Contract Fixtures

`docs/fixtures/mcp-contract.json` is checked by the Rust E2E suite against live MCP stdio responses, install dry-runs, and the CLI reference. Update the fixture and `docs/MCP_CONTRACT.md` together when adding or removing MCP tools, resources, prompts, or install snippets.
