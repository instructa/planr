# Planr Skills

Planr ships agent-facing skill templates under `skills/`.

## Included Skills

- `planr-task-graph`: active task graph coordination with plans, parent gates, map items, picks, runtime state, approvals, logs, reviews, handoffs, stories, and recovery.
- `planr-plan`: product and build planning.
- `planr-work`: one picked item to evidence-backed completion.
- `planr-review`: findings-first review gates.
- `planr-status`: honest read-only status.
- `planr-summary`: evidence-backed summaries.

## Install For Codex

Copy the Planr skills into Codex's local skill directory:

```bash
mkdir -p ~/.codex/skills
cp -R skills/planr-* ~/.codex/skills/
```

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

Claude Code does not use Codex skill folders. Use MCP and paste the Planr workflow prompt into project instructions when needed:

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
