# Planr

![Planr — turn chaotic agent work into a verified task graph](public/planr_banner1.webp)

Planr is a local-first planning and execution coordination tool for coding agents. It combines reviewable Markdown plans with a dependency-aware work map so Codex, Claude Code, Cursor, generic MCP clients, and human operators can drive the same work safely — from idea to verified completion.

[**View the Demo →**](https://x.com/kevinkern/status/2066957434564808884?s=20)

[**Documentation →**](https://planr.so/docs)

```text
idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> close
```

## Why Planr

![Why Planr exists — without Planr vs with Planr](public/planr_banner2.webp)

Flat todo lists break down the moment real work has structure. Planr models work as a dependency graph because that is what work actually is:

- **Readiness is computed, not guessed.** An item becomes `ready` only when its blockers are closed; `planr pick` returns work that is actually startable.
- **Parallel agents need atomic claims.** Picks are atomic claims enforced by the database — one item, one owner, no checklist races.
- **"Done" is gated, not asserted.** Closure requires log-backed evidence (files, commands, tests) and open reviews block their target.
- **State survives sessions.** Markdown plans hold scope and acceptance criteria; the SQLite graph holds live status across handoffs, restarts, and agent switches.
- **Failure is structured.** Stale picks, timeouts, and retries are detectable and recoverable (`planr recover sweep`).

Three layers make that work: **Plans** (reviewable Markdown packages), the **Map** (live dependency graph with picks, reviews, logs), and **Agent loops** (skills, CLI, and MCP workflows for every major coding agent). Full model: [Task Graph Model](docs/TASK_GRAPH_MODEL.md) and [Operating Model](docs/OPERATING_MODEL.md).

## Install

```bash
brew install instructa/tap/planr
```

Or via npm (ships platform-native binaries, no toolchain needed):

```bash
npm install -g planr
```

Or with the release installer:

```bash
curl -fsSL https://raw.githubusercontent.com/instructa/planr/main/scripts/install.sh | sh
```

Then initialize a project. When selected, Claude Code and Cursor also receive standalone project worker/reviewer roles; Codex workflow skills come from its plugin:

```bash
planr project init "My Product" --client all
```

Manual downloads, from-source builds, and client wiring details: [Install Guide](docs/INSTALL.md).

## Install The Plugin (Skills)

The plugin under `plugins/planr` carries the ten Planr workflow skills. Optional model-routing declarations live in repository-local files such as `.planr/agents.toml` and `.planr/policy.toml`; external tools may manage those files, but Planr does not install or invoke a routing engine. The `planr` CLI (above) is required separately.

<a id="install-plugin-codex"></a>
<details>
<summary><strong>Codex</strong></summary>

```bash
codex plugin marketplace add instructa/planr
codex plugin add planr@planr
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

One command installs everything the plugin would carry:

```bash
planr install cursor            # writes .cursor/mcp.json, .cursor/agents/, and .cursor/skills/
planr install cursor --no-mcp   # project skills, subagents, and hooks; no MCP config
```

The dry-run also prints a one-click `cursor://` deeplink for user-level MCP install. Marketplace listing is pending review. Multitasking with Cursor subagents: [Cursor guide](docs/CURSOR.md).

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

## Tell Your Agent

Remember one public entry point: `$planr`. It routes ordinary planning and status work from live Planr state. For long autonomous runs, use the explicit two-step workflow: `$planr-goal` prepares durable state, then `$planr-loop` executes the resulting plan.

Start a new product from an idea:

```text
Use $planr.

Create a production-ready Habit Tracker web app plan. Create the product plan,
split an MVP build plan, check it, then build the Planr map. Do not implement yet.
```

For a long autonomous run, prepare outside the driver first. The preparation result prints a real plan id; Codex or Claude Code then starts only the plan-bound loop driver:

```text
Use $planr-goal to prepare an autonomous goal for the weekly overview feature.

/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr
context (tag: goal-contract).

Goal: ship the weekly overview feature. DONE when every in-scope map item is closed
with log evidence, all reviews are closed complete, and a live verification log shows
the feature working in the browser. Iteration budget: 10.
```

Mid-project work (a new feature, refactor, or fix on an existing project) works the same — it gets its own feature-scoped plan and extends the existing map. Both journeys with example prompts: [Two Journeys](docs/SKILLS.md#two-journeys-new-project-vs-existing-project). Coding agents inspect progress with the compact default `planr map show` or, preferably, `planr map show --json`. The tree preserves exact dependency vocabulary while marking satisfied edges as `blocks✓`; active `blocks` stay red. The boxed `planr map show --view diagram` renderer is exclusively for human supervision and uses neutral `then` routes once those dependencies are satisfied. Agents must not invoke it. Humans can add `--full` for complete status, title, worker, critical-lane, and pressure details. Interactive map output colors states automatically; `--no-color` and `NO_COLOR` keep it plain.

To supervise an agent from a second terminal, leave the agent running in terminal A and watch its scoped graph in terminal B:

```bash
planr map watch --plan <plan-id>
# optional: exit after every scoped item settles
planr map watch --plan <plan-id> --until-settled
# optional: inspect complete node details
planr map watch --plan <plan-id> --full
```

The watcher is likewise a human-only observer. It defaults to the condensed diagram view, polls the local SQLite graph once per second, and redraws only when state changes. Coding agents must not invoke `map watch`; they should use `map show --json` snapshots or the `/v1/events/stream` SSE endpoint instead. Use Ctrl-C to stop.

## What's new

- **1.7.0 — Evidence-backed evaluations:** Added durable eval suites, runs, comparisons, invalidation and rescoring, correctness/quality/performance gates, cost per verified success, and effort recommendations. The complete workflow is available through the CLI; MCP only mirrors selected surfaces and is optional. Security gates now cover repository leaks, vulnerable dependencies, workflow hardening, privacy, and forbidden staged files. See the [Eval Contract](docs/planr-spec/EVAL_CONTRACT_V1.md), [CLI Reference](docs/CLI_REFERENCE.md), and the [1.7.0 changelog](CHANGELOG.md#170---2026-07-22).
- **1.6.0 — Human map observation:** Added a condensed boxed diagram, live two-terminal watching, accessible state colors, and clearer satisfied dependency routes. These views are intentionally for human supervision; agents keep using the default tree or JSON snapshots. See [Task Graph Model](docs/TASK_GRAPH_MODEL.md), [CLI Reference](docs/CLI_REFERENCE.md), and the [1.6.0 changelog](CHANGELOG.md#160---2026-07-21).
- **1.5.2 — Standalone core, optional Switchloom:** Planr consumes provider-neutral repository declarations and route evidence only; it works without any routing files, and requested-only routing metadata is not execution proof. Optional model-routing lifecycle is external, with [Switchloom v0.2.1](https://github.com/instructa/switchloom/releases/tag/v0.2.1) verified as the repository-local handoff outside Planr. Start with [Model Routing](docs/MODEL_ROUTING.md), [Switchloom](https://switchloom.ai), the [Switchloom repository](https://github.com/instructa/switchloom), its tagged [setup quickstart](https://github.com/instructa/switchloom/blob/v0.2.1/README.md#setup-from-the-website) and [lifecycle docs](https://github.com/instructa/switchloom/blob/v0.2.1/docs/preset-composition.md#repository-lifecycle-commands), and the [Changelog](CHANGELOG.md).
- **1.4.0 — Verified presets:** Added policy-driven composition, evaluation, signed registry evidence, and the public catalog. See the [1.4.0 release notes](https://github.com/instructa/planr/releases/tag/v1.4.0).
- **1.3.0 — Native host hooks:** Added automatic session-state injection and loop recovery for supported hosts. See the [Hooks guide](docs/HOOKS.md) and [1.3.0 release notes](https://github.com/instructa/planr/releases/tag/v1.3.0).

For the complete release history, see the [Changelog](CHANGELOG.md).

## Docs

- [Install](docs/INSTALL.md)
- [Skills](docs/SKILLS.md)
- [Long-Running Goals](docs/GOALS.md)
- [Model Routing](docs/MODEL_ROUTING.md) · [Worked Example: Web App](docs/EXAMPLE_WEBAPP.md)
- [Host Hooks](docs/HOOKS.md)
- [CLI Reference](docs/CLI_REFERENCE.md)
- [MCP Guide](docs/MCP_GUIDE.md)
- [Codex](docs/CODEX.md) · [Claude Code](docs/CLAUDE_CODE.md) · [Cursor](docs/CURSOR.md)
- [Operating Model](docs/OPERATING_MODEL.md)
- [Task Graph Model](docs/TASK_GRAPH_MODEL.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Testing](docs/TESTING.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Specification Package](docs/planr-spec/README.md)
- More: [Changelog](CHANGELOG.md), [Import](docs/IMPORT.md), [Security](docs/SECURITY.md), [Handoffs And Stories](docs/HANDOFFS_AND_STORIES.md), [npm Package](docs/NPM.md)

## License

MIT. See `LICENSE.md`.
