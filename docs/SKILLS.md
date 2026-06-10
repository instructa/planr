# Planr Skills

Planr ships agent-facing skill templates under `plugins/planr/skills/`.

The repository ships an installable plugin under `plugins/planr` for Codex, Claude Code, and Cursor, so the skills can be installed as one package instead of copied by hand. Marketplace manifests at the repo root (`.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`) point at that subdirectory — Codex silently ignores marketplaces whose plugin source is the repo root itself. The plugin only carries skills and agent roles; the `planr` CLI must be installed separately (`brew install instructa/tap/planr`).

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

Cursor: pending marketplace review; until listed, use MCP plus the CLI prompt (below).

opencode: no plugin yet; use `planr mcp` as an MCP server (below). A JS plugin wrapping the CLI as custom tools is a possible follow-up.

## Included Skills

Entry points (what users invoke):

- `planr`: master router. One entry point for any request; reads live map state and dispatches to the right skill. Users do not need to remember skill names.
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

Default usage needs two skills:

```text
$planr        any request -> routed to the right stage skill from live map state
$planr-loop   one feature -> loop work/verify/review/fix until done or budget exhausted
```

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

Example autonomous feature loop:

```text
Use $planr-loop.

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

Both journeys use the same entry point (`$planr` or `$planr-loop`). What differs is the state the router finds, and what kind of plan the work gets.

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

The router runs the full stage order: product plan -> build plan -> map -> work. From there, `$planr-loop` or `$planr-work` executes against the map.

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
3. Execution is identical to journey 1: `$planr-loop` for autonomous, `$planr-work` / `$planr-review` for human-in-the-loop.

Or autonomous in one prompt:

```text
Use $planr-loop.

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

`planr-loop` keeps maker and checker separate. Hosts with subagents get dedicated roles that are prompted with skills, not hand-written prompts:

```bash
# Codex: project-scoped agents preloading planr-work / planr-review
cp plugins/planr/skills/planr-loop/agents/*.toml .codex/agents/

# Claude Code standalone (the plugin registers these automatically)
cp plugins/planr/agents/*.md .claude/agents/
```

Dispatches stay one line: `Use $planr-work on item <id>` and `Use $planr-review on item <id>`. The map and logs are the loop memory, so any iteration can resume from zero context.

## Install For Codex

Copy the Planr skills into Codex's local skill directory:

```bash
mkdir -p ~/.codex/skills
cp -R plugins/planr/skills/* ~/.codex/skills/
```

If Planr was installed from an npm package that includes `skills/`, copy from the package location instead:

```bash
PLANR_PKG="$(npm root -g)/planr"
mkdir -p ~/.codex/skills
cp -R "$PLANR_PKG"/plugins/planr/skills/* ~/.codex/skills/
```

Do not present `npx planr` as the primary install path until the npm artifact ships platform-native Planr binaries. Today the normal user path is the GitHub Release installer; npm is a development and consumer-test wrapper.

Then run Codex from a repository where `planr` is installed and initialized:

```bash
planr project init "Example Product" --client codex
planr doctor --client codex
```

Codex can also use Planr through MCP:

```bash
planr install codex --dry-run
planr prompt mcp --client codex
```

## Install For Claude Code

Claude Code loads project skills from `.claude/skills/`:

```bash
mkdir -p .claude/skills .claude/agents
cp -R plugins/planr/skills/* .claude/skills/
cp plugins/planr/agents/*.md .claude/agents/
```

Then add MCP and the Planr workflow prompt to project instructions when needed:

```bash
planr project init "Example Product" --client claude
planr install claude --dry-run
planr prompt mcp --client claude
planr prompt cli --client claude
```

`planr install claude` writes a project-scoped `.mcp.json` when not run as a dry-run.

## Install For Cursor

Cursor should use Planr through MCP plus the CLI prompt:

```bash
planr project init "Example Product" --client cursor
planr install cursor --dry-run
planr prompt mcp --client cursor
planr prompt cli --client cursor
```

`planr install cursor` writes `.cursor/mcp.json` when not run as a dry-run. Use `planr serve --port 7526` and `planr prompt http --client cursor` if a Cursor workflow should inspect the local HTTP/review workspace.

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
planr pick heartbeat <item-id>
planr approval list --open
planr log add --item <item-id> --summary "..." --cmd "..."
planr review request <item-id>
planr close <item-id> --summary "Verified"
```

See also:

- [Operating Model](OPERATING_MODEL.md)
- [Task Graph Model](TASK_GRAPH_MODEL.md)
- [Handoffs And Stories](HANDOFFS_AND_STORIES.md)
