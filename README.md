# Planr

![Planr workflow: plan -> fix -> review against repo state](public/planr_workflow.png)

Planr is a local-first task planning and execution coordination tool for coding agents. It combines reviewable Markdown plans with a dependency-aware work map so Codex, Claude Code, Cursor, and other MCP-capable clients can coordinate safely.

## Product Direction

Planr combines three layers:

- **Plans:** reviewable Markdown artifacts for product specs, PRDs, architecture, UX, build contracts, and reviews.
- **Map:** the live dependency graph for items, links, picks, reviews, logs, and progress.
- **Agent loops:** provider-neutral CLI and MCP workflows for Codex, Claude Code, Cursor, and generic clients.

The product flow is:

```text
idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close
```

Planr ships as a self-owned Rust CLI with SQLite graph storage, Markdown plan packages, recovery sweeps, scoped Git review evidence, reusable packages, MCP stdio integration, prompt output, and a localhost HTTP/SSE surface with a browser review workspace.

## Quick Start

Install the current GitHub Release:

```bash
curl -fsSL https://raw.githubusercontent.com/instructa/planr/main/scripts/install.sh | sh
```

Manual release downloads are published at:

- https://github.com/instructa/planr/releases

Homebrew is the preferred package-manager path once the tap is published:

```bash
brew install instructa/tap/planr
```

After install:

```bash
planr project init "Example Product" --client all
planr plan new "Inventory API" --platform api --backend
planr plan split <plan-id> --slice "MVP backend"
planr map build --from <build-plan-id>
planr pick
planr log add --item <item-id> --summary "Implemented first slice" --files src/api.rs --cmd "cargo test"
planr review request <item-id>
planr review evidence <item-id>
planr review close <review-id> --verdict complete
planr close <item-id> --summary "Verified"
```

Use `--json` for automation and `--db <path>` for isolated databases.

## From Source

Maintainers and contributors can run the CLI directly from the Rust workspace:

```bash
cargo run -- project init "Example Product" --client all
cargo run -- plan new "Inventory API" --platform api --backend
cargo run -- plan split <plan-id> --slice "MVP backend"
cargo run -- map build --from <build-plan-id>
cargo run -- pick
cargo run -- log add --item <item-id> --summary "Implemented first slice" --files src/api.rs --cmd "cargo test"
cargo run -- review request <item-id>
cargo run -- review close <review-id> --verdict complete
cargo run -- close <item-id> --summary "Verified"
```

## Target Agents

- Codex
- Claude Code
- Cursor
- Generic MCP clients
- Human operators using the CLI

## Current Repository Contents

- `src/main.rs`: process composition root and CLI startup.
- `src/cli.rs`: typed CLI command definitions.
- `src/app/`: command orchestration, MCP, HTTP/SSE, repository helpers, review gates, runtime surfaces, and local inspection flows.
- `src/storage/`: SQLite path, connection, schema, and row-mapping ownership.
- `src/planpack.rs`: project and product-plan Markdown package generation.
- `src/model.rs`: serializable Planr DTOs shared across CLI, JSON, MCP, and HTTP responses.
- `src/integrations.rs`: Codex, Claude Code, Cursor, and MCP install descriptors.
- `src/util.rs`: small CLI-boundary helpers for ids, paths, output, and file writes.
- `tests/e2e.rs`: real CLI, MCP, HTTP, import, review-gate, run-log, and concurrent-pick tests.
- `skills/planr-*`: public Planr-native skill templates for coding agents.
- `docs/planr-spec/`: production-ready product specification package for Planr V1.
- `examples/real-world-flow.md`: executable real-world operator flow.
- `public/planr_workflow.png`: existing workflow image.

## Integrations

```bash
planr doctor --client all
planr install codex --dry-run
planr install claude --dry-run
planr install cursor --dry-run
planr prompt mcp
planr prompt cli --client codex
planr prompt http
planr mcp
planr serve --port 8484
```

`planr install claude` writes a project `.mcp.json`; `planr install cursor` writes `.cursor/mcp.json`; dry-runs print the exact config and scope notes first.

Open `http://127.0.0.1:8484/review` after `planr serve` for the local browser review workspace.

## Verification

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Release-grade verification also covers the checksum installer, MCP contract fixture, browser review workspace, recovery sweep, package export/import, prompt output, and a fresh consumer E2E project at `~/projects/planr-test`.

## Specification Package

Start here:

- [docs/planr-spec/README.md](docs/planr-spec/README.md)
- [docs/planr-spec/PRODUCT_SPEC.md](docs/planr-spec/PRODUCT_SPEC.md)
- [docs/planr-spec/TECH_ARCHITECTURE.md](docs/planr-spec/TECH_ARCHITECTURE.md)
- [docs/planr-spec/API_AND_DATA_MODEL.md](docs/planr-spec/API_AND_DATA_MODEL.md)
- [docs/planr-spec/TASKS.md](docs/planr-spec/TASKS.md)

The zip artifact is at:

- [docs/planr-spec.zip](docs/planr-spec.zip)

## Guides

- [Install](docs/INSTALL.md)
- [CLI Reference](docs/CLI_REFERENCE.md)
- [MCP Guide](docs/MCP_GUIDE.md)
- [npm Package](docs/NPM.md)
- [Codex](docs/CODEX.md)
- [Claude Code](docs/CLAUDE_CODE.md)
- [Cursor](docs/CURSOR.md)
- [Import](docs/IMPORT.md)
- [Security](docs/SECURITY.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Operating Model](docs/OPERATING_MODEL.md)
- [Task Graph Model](docs/TASK_GRAPH_MODEL.md)
- [Handoffs And Stories](docs/HANDOFFS_AND_STORIES.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Testing](docs/TESTING.md)
- [Skills](docs/SKILLS.md)

## License

MIT. See `LICENSE.md`.
