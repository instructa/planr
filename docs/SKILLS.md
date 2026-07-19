# Planr Skills

Planr ships agent-facing skill templates under `plugins/planr/skills/`.

The repository ships an installable plugin under `plugins/planr` for Codex and Claude Code, while Cursor receives the same skills through `planr install cursor`. Marketplace manifests at the repo root (`.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`) point at that subdirectory — Codex silently ignores marketplaces whose plugin source is the repo root itself. The shared package carries skills and Claude's independent workflow roles; optional model-specific host roles come from repository-local routing declarations managed outside Planr, never from Planr Core or static fallbacks. The `planr` CLI must be installed separately (`brew install instructa/tap/planr`).

## Install As Plugin (preferred)

Codex:

```bash
codex plugin marketplace add instructa/planr
# then install "planr" from the plugin directory picker, or:
codex plugin add planr@planr
```

Claude Code:

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

Skills are namespaced in Claude Code: `/planr:planr`, `/planr:planr-loop`. The plugin also registers the `planr-worker` and `planr-reviewer` subagents from the plugin's `agents/` directory.

Cursor: pending marketplace review; `planr install cursor` provides the identical component set today — it writes the skills to `.cursor/skills/`, the subagents to `.cursor/agents/`, and the MCP config in one command (see below and [Cursor](CURSOR.md)).

opencode: no plugin yet; use `planr mcp` as an MCP server (below). A JS plugin wrapping the CLI as custom tools is a possible follow-up.

## Included Skills

Entry points (what users invoke):

- `planr`: master router. One entry point for any request; reads live map state and dispatches to the right skill. Users do not need to remember skill names.
- `planr-goal`: goal prep compiler for long-running runs. Turns a broad goal into a checked plan, a linked map, and a durable goal contract (`planr context --tag goal-contract`), then prints the starter command for the host's loop driver (Codex/Claude Code `/goal`, or manual re-dispatch). Prep only — see [Long-Running Goals](GOALS.md).
- `planr-loop`: autonomous closing loop. Drives one feature to verified completion — work, live verification, independent review, fix items — until the map is clean or the iteration budget runs out. Ships subagent templates under `plugins/planr/skills/planr-loop/agents/`.

Capability skills (dispatched by the loop's live-verification step):

- `planr-verify-web`: proves a web feature runs in a browser. Discovers the host's existing browser capability (browser skill, browser MCP, `npx playwright`, HTTP checks as last resort), records the choice as a `capability` context, and logs replayable evidence. Ships no browser tooling itself.

Stage skills (what the router and loop dispatch to; also directly invocable):

- `planr-task-graph`: active task graph coordination with plans, parent gates, map items, picks, runtime state, approvals, logs, reviews, handoffs, stories, and recovery.
- `planr-plan`: product and build planning.
- `planr-work`: one picked item to evidence-backed completion.
- `planr-review`: findings-first review gates.
- `planr-status`: honest read-only status.
- `planr-summary`: evidence-backed summaries.

## Cheat Sheet

Default usage needs one public entry point:

```text
$planr        any request -> routed to the right stage skill from live map state
```

`$planr-goal` and `$planr-loop` are advanced stage surfaces selected by the router. A long-running goal is always prepared first; only the resulting real plan id is passed to the loop driver.

The stage order the router follows for a new app:

```text
$planr-plan        idea -> product plan -> build plan
$planr-task-graph  build plan -> map -> dependencies -> critical lane
$planr-work        pick one ready item -> implement -> log evidence -> request review
$planr-review      audit evidence -> complete or create fix work
$planr-work        pick generated fix work when review finds issues
$planr-status      report honest state, blockers, and next ready work
$planr-summary     summarize completed scope with evidence
```

Example first prompt for a Habit Tracker:

```text
Use $planr.

Create a production-ready Habit Tracker web app plan. Include habits, daily check-ins,
streaks, weekly overview, local-first persistence, tests, privacy, and release readiness.
Create the product plan, split an MVP build plan, check it, then build the Planr map.
Do not implement yet. End with the build plan id, critical lane, and first ready items.
```

Example autonomous feature loop (two separate prompts):

```text
Use $planr-goal to prepare an autonomous goal for the weekly overview feature.

/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr
context (tag: goal-contract).

Goal: ship the weekly overview feature. DONE when every in-scope map item is closed with
log evidence, all reviews are closed complete, and a live verification log shows the
overview rendering real check-in data in the browser. Iteration budget: 10.
```

Example single implementation step (human-in-the-loop):

```text
Use $planr-work.

Pick exactly one ready Habit Tracker item. Implement only that item, keep Planr runtime
state current, log changed files and real verification commands, then request review.
Do not close the item until review is complete.
```

## Two Journeys: New Project vs. Existing Project

Both journeys use the same public entry point (`$planr`). What differs is the state the router finds, and what kind of plan the work gets.

### Journey 1 — start a project from an idea

Initialize once per repository, then hand the idea to the router:

```bash
planr project init "Habit Tracker" --client all
```

```text
Use $planr.

Create a production-ready Habit Tracker web app plan. Create the product plan,
split an MVP build plan, check it, then build the Planr map. Do not implement yet.
```

The router runs the full stage order: product plan -> build plan -> map. From there it can select `$planr-work`, or prepare a plan-bound `$planr-loop` run.

### Journey 2 — mid-project: add a feature, refactor, or fix

Never re-run `project init`; the project and map already exist. Every new scope — a feature like an auth system, a refactor, a non-trivial fix — gets its own feature-scoped plan on the same map:

```text
Use $planr.

Add an auth system (email+password, sessions, protected routes) to this app.
Create a feature plan for it, record what existing code it builds on, split a
narrow build slice, check it, and extend the map with linked items. Do not implement yet.
```

What the router does with that, and why:

1. `$planr-plan` creates a new plan scoped to the feature (`planr plan new "Auth system" ...`), not a new project. Refine notes capture constraints from the existing codebase; the build plan's "existing leverage" field records what is reused instead of rebuilt.
2. `$planr-task-graph` extends the existing map: new items, plus `blocks` links to anything already on the map that must land first.
3. Execution is identical to journey 1: a plan-bound `$planr-loop` for autonomous work, or `$planr-work` / `$planr-review` for human-in-the-loop.

Or autonomous in two prompts:

```text
Use $planr-goal to prepare an autonomous goal for the auth system.

/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr
context (tag: goal-contract).

Goal: ship an auth system (email+password, sessions, protected routes).
DONE when every auth map item is closed with log evidence, all reviews are closed
complete, and a live verification log shows login and a protected route working
in the browser. Iteration budget: 10.
```

Rules that hold in both journeys:

- No map items without a checked build plan — even a small fix gets a minimal slice (`plan new` -> `plan split` with a tiny scope). This keeps closure evidence and reviews attached to a contract.
- Plans accumulate: `planr plan list` shows the project's history of scopes; the map stays the single live source of item status.
- Status, review, and summary requests (`$planr-status`, `$planr-review`, `$planr-summary`) work the same at any point in either journey.

## Loop Roles

`planr-loop` keeps maker and checker separate. Hosts with subagents get dedicated roles that are prompted with skills, not hand-written prompts.

The CLI provisions the role files automatically — no manual copying:

```bash
planr project init "My Product" --client all   # writes standalone Claude and Cursor roles; Codex has no project roles
planr agents init                              # writes the provider-neutral .planr/agents.toml registry; it does not generate Codex roles
planr install claude                           # provisions Claude's independent roles
planr install cursor                           # provisions Cursor's independent roles and skills
```

Optional project-scoped model-routing files are repository-local declarations. They may be edited directly or managed by an external tool such as [Switchloom v0.2.1](https://github.com/instructa/switchloom/releases/tag/v0.2.1); Core workflow skills remain host-neutral, and Planr does not install, invoke, apply, or uninstall external routing artifacts.

Dispatches stay one line: `Use $planr-work on item <id>` and `Use $planr-review on item <id>`. The map and logs are the loop memory, so any iteration can resume from zero context.

## Install For Codex

Install the Codex plugin for all ten workflow skills, then initialize and install the project integration:

```bash
codex plugin marketplace add instructa/planr
codex plugin add planr@planr
planr project init "Example Product" --client codex
planr install codex
planr doctor --client codex
```

The CLI writes the project MCP snippet and hooks. It does not copy project skills or agents; those skills are plugin-owned, and Codex has no Planr project-agent contract. `--no-mcp` leaves hooks only, while `--no-mcp --no-hooks` writes neither integration artifact.

## Install For Claude Code

Install the Claude Code plugin for all ten workflow skills and its plugin worker/reviewer agents, then install the project integration:

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

```bash
planr project init "Example Product" --client claude
planr install claude
planr doctor --client claude
```

The CLI writes project-scoped `.mcp.json`, standalone project worker/reviewer roles, and hooks. It does not copy project skills. `--no-mcp` retains the standalone roles and hooks; add `--no-hooks` to omit hooks.

## Install For Cursor

One command wires everything — MCP, the skills, and the subagent roles:

```bash
planr project init "Example Product" --client cursor
planr install cursor
```

`planr install cursor` writes `.cursor/mcp.json`, copies the ten skills to `.cursor/skills/`, provisions `.cursor/agents/planr-worker.md` and `planr-reviewer.md`, reconciles hooks, and prints a one-click deeplink for user-level MCP install. `planr install cursor --no-mcp` retains the agents, skills, and hooks while omitting MCP; add `--no-hooks` to omit hooks. Invoke the public router with `/planr` in Agent chat, and dispatch subagents with `/planr-worker` and `/planr-reviewer`. Use `planr serve --port 7526` and `planr prompt http --client cursor` if a Cursor workflow should inspect the local HTTP/review workspace. Subagent multitasking and worktree guidance: [Cursor](CURSOR.md).

## MCP-Only Clients

Any MCP-capable coding agent can run:

```bash
planr mcp
```

Use these commands for setup text without editing global config:

```bash
planr prompt mcp --client all
planr prompt cli --client all
planr prompt http --client all
```

## What The Skills Do

The skills are client-neutral and use only Planr-owned commands:

```bash
planr project show --json
planr plan new "App idea"
planr map build --from <plan-id>
planr pick --json
planr done <item-id> --summary "..." --files a --files b --cmd "..." --review --next
planr review close <review-id> --verdict complete --close-target
planr approval list --open
```

The granular commands (`log add`, `review request`, `close`, `pick heartbeat`) remain available; `done` chains them with identical evidence.

See also:

- [Operating Model](OPERATING_MODEL.md)
- [Task Graph Model](TASK_GRAPH_MODEL.md)
- [Handoffs And Stories](HANDOFFS_AND_STORIES.md)
