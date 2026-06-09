# Planr Skills

Planr ships agent-facing skill templates under `skills/`.

## Included Skills

- `planr-task-graph`: active task graph coordination with plans, parent gates, map items, picks, runtime state, approvals, logs, reviews, handoffs, stories, and recovery.
- `planr-plan`: product and build planning.
- `planr-work`: one picked item to evidence-backed completion.
- `planr-review`: findings-first review gates.
- `planr-status`: honest read-only status.
- `planr-summary`: evidence-backed summaries.

## Install Locally For Codex-Style Skill Runtimes

Copy the wanted skill folder into your runtime's skill directory, for example:

```bash
mkdir -p ~/.codex/skills
cp -R skills/planr-task-graph ~/.codex/skills/
```

The skills use only Planr-owned commands:

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

They are client-neutral and work with Codex, Claude Code, Cursor, or any MCP-capable coding agent that can run shell commands.

See also:

- [Operating Model](OPERATING_MODEL.md)
- [Task Graph Model](TASK_GRAPH_MODEL.md)
- [Handoffs And Stories](HANDOFFS_AND_STORIES.md)
