# Planr

Planr is a local-first task planning and execution coordination tool for coding agents. It combines reviewable Markdown plans with a dependency-aware work map so Codex, Claude Code, Cursor, and other MCP-capable clients can coordinate safely.

## Table of Contents

- [Why a Graph-Based Planner](#why-a-graph-based-planner)
- [Product Direction](#product-direction)
- [Quick Start](#quick-start)
- [From Source](#from-source)
- [Target Agents](#target-agents)
- [Current Repository Contents](#current-repository-contents)
- [Install The Plugin (Skills)](#install-the-plugin-skills)
  - [Codex](#install-plugin-codex)
  - [Claude Code](#install-plugin-claude-code)
  - [Cursor](#install-plugin-cursor)
  - [opencode](#install-plugin-opencode)
- [Integrations](#integrations)
- [Verification](#verification)
- [Specification Package](#specification-package)
- [Guides](#guides)
- [License](#license)

## Why a Graph-Based Planner

Flat todo lists and Markdown checklists break down the moment real work has structure. Planr models work as a dependency graph because that is what work actually is:

- **Readiness is computed, not guessed.** A flat list cannot express "B cannot start before A is closed." In the graph, an item becomes `ready` only when its blockers are closed, so an agent never has to interpret a checklist — it asks `planr pick next` and gets work that is actually startable.
- **Parallel agents need atomic claims.** Two agents editing the same checklist is a race condition. Picks are atomic claims on ready items: one item, one owner, enforced by the database, not by convention.
- **The critical path is a query.** "What unblocks the most downstream work?" and "what is the longest remaining chain?" are unanswerable with a list. With a graph they are one command: `planr map lane --critical` and `planr map pressure`.
- **"Done" is gated, not asserted.** Closure requires log-backed evidence (files changed, commands run, tests) and open review items block their target. A checked checkbox proves nothing; a closed graph node carries its proof.
- **Nesting gives context without losing state.** Markdown plans hold scope, acceptance criteria, and narrative; the graph holds live status. Chat history evaporates between sessions — the graph survives handoffs, restarts, and agent switches.
- **Failure is structured.** Stale picks, timeouts, and retries are detectable and recoverable (`planr recover sweep`) because state lives in a database instead of someone's memory.

The result: multiple coding agents can work the same repository concurrently without stepping on each other, and a human can audit at any time what is done, what is blocked, and why. See `docs/TASK_GRAPH_MODEL.md` and `docs/OPERATING_MODEL.md` for the full model.

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

Release assets for darwin/linux on arm64/x86_64 are built and published automatically by the tag-driven pipeline described in `docs/RELEASE.md`.

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
- `plugins/planr/`: the installable plugin payload — all nine skills, the worker and reviewer subagent roles, and the per-host plugin manifests.
- `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`: marketplace manifests pointing Codex and Claude Code at `plugins/planr`.
- `docs/planr-spec/`: production-ready product specification package for Planr V1.
- `examples/real-world-flow.md`: executable real-world operator flow.

## Install The Plugin (Skills)

The repository ships an installable plugin for Codex, Claude Code, and Cursor under `plugins/planr`. The plugin carries the nine Planr skills plus the `planr-worker` and `planr-reviewer` subagent roles; the `planr` CLI itself must be installed separately (see [Quick Start](#quick-start)).

<a id="install-plugin-codex"></a>
<details>
<summary><strong>Codex</strong></summary>

```bash
codex plugin marketplace add instructa/planr
codex plugin add planr@planr
```

Then invoke skills directly in a session:

```text
$planr build me a habit tracker web app
$planr-loop ship the export feature until verified
```

</details>

<a id="install-plugin-claude-code"></a>
<details>
<summary><strong>Claude Code</strong></summary>

Inside a Claude Code session:

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

Restart Claude Code afterwards. Skills are namespaced (`/planr:planr`, `/planr:planr-loop`), and the plugin registers the `planr-worker` and `planr-reviewer` subagents automatically.

</details>

<a id="install-plugin-cursor"></a>
<details>
<summary><strong>Cursor</strong></summary>

Pending marketplace review. Until the plugin is listed, wire Planr in via MCP and the CLI prompt:

```bash
planr install cursor        # writes .cursor/mcp.json
planr prompt cli --client cursor
```

</details>

<a id="install-plugin-opencode"></a>
<details>
<summary><strong>opencode</strong></summary>

No plugin yet. Use Planr as an MCP server and paste the CLI prompt into your agent instructions:

```bash
planr mcp                   # stdio MCP server
planr prompt cli
```

</details>

After install, drive everything through two skills: `$planr <any request>` routes to the right stage skill from live map state, and `$planr-loop` drives one feature through work, live verification, and independent review until the map is clean.

This works the same whether you are starting from an idea or adding to an existing project — a new feature, refactor, or fix on a running project gets its own feature-scoped plan and extends the existing map. Both journeys with example prompts: [Two Journeys in docs/SKILLS.md](docs/SKILLS.md#two-journeys-new-project-vs-existing-project).

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
planr serve --port 7526
```

`planr install claude` writes a project `.mcp.json`; `planr install cursor` writes `.cursor/mcp.json`; dry-runs print the exact config and scope notes first.

Open `http://127.0.0.1:7526/review` after `planr serve` for the local browser review workspace.

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
