# Planr

![Planr — turn chaotic agent work into a verified task graph](public/planr_banner1.webp)

Planr is a local-first planning and execution coordination tool for coding agents. It combines reviewable Markdown plans with a dependency-aware work map so Codex, Claude Code, Cursor, generic MCP clients, and human operators can drive the same work safely — from idea to verified completion.

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

## New in 1.4.0: Verified Presets & Catalog

Planr now ships a verified preset system for composing a provider-neutral usage policy with a host binding, previewing every repository-local change, and applying the result only after confirmation:

```bash
planr agents preset list
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --preview
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --confirm
```

The built-in catalog includes four policies, five host bindings, and 20 declared safe pairs. Codex has one current topology: repository-owned GPT-5.6 Sol, Terra, and Luna roles selected through native `agent_type` dispatch. Reproducible evaluation and the public [Planr Preset Catalog](https://planr-test-catalog.office-35d.workers.dev/) keep evidence inspectable without making the registry a runtime dependency. The native Codex entry remains visibly experimental and unrecommended until a fresh independently signed live oracle passes every objective gate. Full guides: [Preset Composition](docs/PRESET_COMPOSITION.md) · [Preset Evaluation](docs/PRESET_EVALUATION.md) · [Preset Registry](docs/PRESET_REGISTRY.md).

## New in 1.3.0: Native Host Hooks

`planr install codex|claude|cursor` now wires Planr into the host's native hook system by default — every new session (including post-compaction restarts) gets one compact state block injected automatically:

```text
## planr state
project: Hookboard | map: 5/5 settled | 0 ready, 0 picked, 0 in_review
goal contract: DONE when every in-scope map item is closed with log evidence, ...
routing: registry active (3 profiles; pick packets carry model routing)
next: planr plan audit pln-fc584c28 --json
```

- **`planr prime`** — the state block behind the hooks: project, map counts, held items with log status, goal contract, and the next command. Silent in repos without a Planr database.
- **Loop recovery becomes mechanism, not discipline** — an agent that restarts or compacts mid-loop picks up exactly where the map says it left off.
- **Evidence guard (Cursor)** — a subagent that stops while its own pick has no completion log gets one advisory reminder naming the item and the two ways out.
- **Fail-open and additive** — hooks never block a session (10s timeout, always exit 0), existing hook files are merged, `--no-hooks` opts out.

Full guide: [Hooks](docs/HOOKS.md) · [Release notes](https://github.com/instructa/planr/releases/tag/v1.3.0).

## Model Routing (1.2.0)

Declare once which model handles which work — every task then carries its own routing, and your agents delegate automatically:

```toml
# .planr/agents.toml  (write it with `planr agents init`)
[profiles.worker]
client = "codex"
model = "gpt-5.6-terra"
agent_type = "planr-terra-high"
effort = "high"
skill = "planr-work"

[[routes]]
match = { work_type = "code" }
profile = "worker"
```

- **Routing travels in the pick packet** — `planr pick --json` hands the worker its profile, model, and paired skill; `planr pick --peek` lets dispatching drivers read it without taking the lease.
- **Rendered into native config** — the default `agents init` atomically writes the canonical repository-local Codex roles; verified preset apply owns subsequent policy composition. `planr install claude|cursor` retains independent role rendering.
- **Declared vs. actual, with receipts** — workers report the profile they ran on, runs record the observed host, and `planr trace item` shows deviations as advisory markers.
- **Use-case pools** — free-form work types (`frontend`, `backend`, ...) declared right in the plan's task list (`### TASK-001 (backend): ...`), plus per-item pins via `planr item route`.

Routing is advisory by design: Planr never dispatches models and never blocks a pick — hosts stay the authority. Full guide: [Model Routing](docs/MODEL_ROUTING.md) · replayable walkthrough: [Worked Example: Web App](docs/EXAMPLE_WEBAPP.md) · [Release notes](https://github.com/instructa/planr/releases/tag/v1.2.0).

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

Then initialize a project (also provisions the worker/reviewer subagent roles for your client):

```bash
planr project init "My Product" --client all
```

Manual downloads, from-source builds, and client wiring details: [Install Guide](docs/INSTALL.md).

## Install The Plugin (Skills)

The plugin under `plugins/planr` carries the ten Planr skills. Claude Code also consumes its independent `planr-worker` and `planr-reviewer` roles; Cursor receives its own roles through `planr install cursor`. Codex roles are generated only from the canonical native preset into the repository. The `planr` CLI (above) is required separately.

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
planr install cursor --no-mcp   # skills and subagents only, no MCP config
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

Two skills drive everything. `$planr` routes any request to the right stage skill from live map state; `$planr-loop` drives one feature through work, live verification, and independent review until the map is clean.

Start a new product from an idea:

```text
Use $planr.

Create a production-ready Habit Tracker web app plan. Create the product plan,
split an MVP build plan, check it, then build the Planr map. Do not implement yet.
```

Ship one feature autonomously until verified:

```text
Use $planr-loop.

Goal: ship the weekly overview feature. DONE when every in-scope map item is closed
with log evidence, all reviews are closed complete, and a live verification log shows
the feature working in the browser. Iteration budget: 10.
```

Mid-project work (a new feature, refactor, or fix on an existing project) works the same — it gets its own feature-scoped plan and extends the existing map. Both journeys with example prompts: [Two Journeys](docs/SKILLS.md#two-journeys-new-project-vs-existing-project). Watch progress anytime with `planr map show`.

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
