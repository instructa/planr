# Planr Skills

Planr ships agent-facing skill templates under `skills/`.

## Included Skills

Entry points (what users invoke):

- `planr`: master router. One entry point for any request; reads live map state and dispatches to the right skill. Users do not need to remember skill names.
- `planr-loop`: autonomous closing loop. Drives one feature to verified completion — work, live verification, independent review, fix items — until the map is clean or the iteration budget runs out. Ships subagent templates under `skills/planr-loop/agents/`.

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

## Loop Roles

`planr-loop` keeps maker and checker separate. Hosts with subagents get dedicated roles that are prompted with skills, not hand-written prompts:

```bash
# Codex: project-scoped agents preloading planr-work / planr-review
cp skills/planr-loop/agents/codex/*.toml .codex/agents/

# Claude Code: subagents preloading the same skills via frontmatter
cp skills/planr-loop/agents/claude/*.md .claude/agents/
```

Dispatches stay one line: `Use $planr-work on item <id>` and `Use $planr-review on item <id>`. The map and logs are the loop memory, so any iteration can resume from zero context.

## Install For Codex

Copy the Planr skills into Codex's local skill directory:

```bash
mkdir -p ~/.codex/skills
cp -R skills/* ~/.codex/skills/
```

If Planr was installed from an npm package that includes `skills/`, copy from the package location instead:

```bash
PLANR_PKG="$(npm root -g)/planr"
mkdir -p ~/.codex/skills
cp -R "$PLANR_PKG"/skills/* ~/.codex/skills/
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
cp -R skills/* .claude/skills/
cp skills/planr-loop/agents/claude/*.md .claude/agents/
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

`planr install cursor` writes `.cursor/mcp.json` when not run as a dry-run. Use `planr serve --port 8484` and `planr prompt http --client cursor` if a Cursor workflow should inspect the local HTTP/review workspace.

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
