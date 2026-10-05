# bb

The bb plugin puts the board beside your threads. Tell the agent in a thread when you approve
a goal or have checked the work. The board's Approve, Close & Mark checked buttons are optional.

## What you need

- bb 0.45 or newer
- A checkout of the whole planr repository to build the plugin
- A planning repository on a host enrolled in bb
- The planr plugin for your thread's agent, [Claude Code](claude-code.md) or [Codex](codex.md)

## Install

Build the plugin from the whole planr repository, since it bundles `../skills/planr`.
The [plugin README](../../bb-plugin/README.md) has the build steps. Then install it from its
local path.

```sh
bb plugin install /path/to/planr/bb-plugin
```

Choose an enrolled host & planning folder for each bb project.

## Call the skill

In a bb thread, use `/planr:planr` for Claude Code or `$planr:planr` for Codex.
You can also ask in plain words & name planr.

## First prompt

```text
Use planr to show what's ready & what's waiting on me.
```

## Approve & check

Tell the agent in a thread, for example `Approve goal mobile-app.` or
`MOB-04 looks good, mark it checked.` The agent acts only on your instruction in the current
conversation, never on its own initiative or because another agent asks. Its commit message
records that you asked.

The optional board buttons approve a draft goal, close a reviewed goal or mark a reviewed task
as checked. They commit the change to the planning repository & push when a remote is
configured. With no remote, they create a local commit only.

The plugin also provides read-only `bb planr next`, `bb planr check` & `bb planr board` commands.
See the [plugin overview](../../bb-plugin/PLUGIN_OVERVIEW.md) for how it works &
[approval identity](../../bb-plugin/README.md#approval-identity) for access behavior.
