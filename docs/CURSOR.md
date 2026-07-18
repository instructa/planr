# Cursor Integration

Cursor is a first-class Planr client. One command wires a project completely — MCP tools, the worker/reviewer subagents, and the full skill set:

```bash
planr install cursor
```

This writes only repository-local files (never global Cursor config):

| Path | Purpose |
| --- | --- |
| `.cursor/mcp.json` | project-scoped stdio MCP server (`planr mcp`) with the 44 Planr tools |
| `.cursor/agents/planr-worker.md` | worker subagent: implements exactly one picked item, then requests review and stops |
| `.cursor/agents/planr-reviewer.md` | reviewer subagent: audits evidence and closes the review with a verdict |
| `.cursor/skills/planr*/SKILL.md` | all ten Planr skills (`planr`, `planr-goal`, `planr-loop`, stage and capability skills) |

Re-running the command refreshes `.cursor/mcp.json` but never overwrites edited agent or skill files. `planr project init "My Product" --client cursor` (or `--client all`) provisions the same subagent roles at init time. Verify with `planr doctor --client cursor`.

## Skills And Agents Only (No MCP)

The skills are CLI-first — they drive the `planr` binary directly through the terminal — so MCP is optional. To install project subagents, skills, and the default hooks without MCP:

```bash
planr install cursor --no-mcp
```

This writes `.cursor/agents/`, `.cursor/skills/`, and the default project hooks, but no `.cursor/mcp.json` or deeplink. Add `--no-hooks` when hooks must also be skipped. Everything below works in this mode because the skills call the Planr CLI directly. `--no-mcp --dry-run` lists the roles and skills and states whether the non-dry run would reconcile hooks. For Claude Code the flag writes standalone project roles and hooks but no project skills; for Codex it writes hooks only. Their workflow skills come from their respective plugins.

## One-Click MCP Install

For user-level setup (all projects, no repo files), the dry-run prints a Cursor deeplink:

```bash
planr install cursor --dry-run
```

```text
cursor://anysphere.cursor-deeplink/mcp/install?name=planr&config=eyJhcmdzIjpbIm1jcCJdLCJjb21tYW5kIjoicGxhbnIifQ==
```

Open the link (or paste it into a browser) and Cursor prompts to install the server. The embedded config is `{"command": "planr", "args": ["mcp"]}` with no `--db` path on purpose: Cursor starts the server in the workspace directory, so every project resolves its own `.planr/planr.sqlite`.

## Plugin

The repository carries a Cursor plugin manifest (`.cursor-plugin/plugin.json`) bundling the skills and both subagents. Marketplace listing is pending review; until it is listed:

- `planr install cursor` (above) provides the identical component set per project, or
- install the plugin locally: copy the repository to `~/.cursor/plugins/local/planr` and reload Cursor.

## Multitasking With Cursor's Built-In Features

Cursor's subagents, background execution, and parallel agents map directly onto Planr's maker/checker loop. The Planr map is the shared state, so concurrent Cursor agents coordinate through `planr pick` leases instead of stepping on each other.

### Subagent dispatch (maker/checker)

The installed subagents are dispatched from any Agent chat with `/name` or by mention:

```text
/planr-worker implement item <item-id>
/planr-reviewer review item <item-id>
```

The worker preloads the `planr-work` protocol (implement one item, log evidence, request review, stop); the reviewer preloads `planr-review` (audit the diff, rerun logged commands, close with a verdict). Because each subagent runs in its own context window, the driving chat stays focused on `plan audit`, dispatch decisions, and synthesis — the same division of labor as Codex `/goal` and Claude Code (see [Long-Running Goals](GOALS.md)).

Cost tiering: the worker file ships with `model: inherit`; edit its frontmatter to pin a cheaper Cursor model id when dispatch cost matters. Leave the reviewer on `inherit` — make workers cheap, not the verdict.

### Parallel and background work

- **Parallel subagents:** ask the driver to dispatch several workers at once ("implement items A, B, and C in parallel with planr-worker subagents"). Each worker calls `planr pick` (or is handed its item id); the map's single-owner lease guarantees no two agents hold the same item, and `planr recover sweep` reclaims work from any that die.
- **Background subagents:** long items can run as background subagents; evidence logging doubles as the liveness heartbeat, so `planr pick stale` shows honestly stuck work.
- **Cloud agents (`/in-cloud`):** cloud VMs do not share your local `.planr` database or local MCP config. Give cloud agents the CLI workflow instead (`planr prompt cli --client cursor`) and let them commit evidence through normal Planr CLI calls in their own checkout.

### Parallel agents in worktrees

Cursor's parallel agents run in separate git worktrees, and each worktree would otherwise get its own copy of `.planr/planr.sqlite` — forking the map. Keep one authoritative map by pointing every worktree at a shared absolute database path:

```json
{
  "mcpServers": {
    "planr": {"command": "planr", "args": ["--db", "/absolute/path/to/main-checkout/.planr/planr.sqlite", "mcp"]}
  }
}
```

`planr install cursor` already writes the absolute path of the current checkout, so worktrees created from an installed project inherit the right value — verify with `planr project show --json` from each worktree. One picked item per agent instance stays the rule (`plugins/planr/skills/planr-loop/SKILL.md`, Hard Rules).

### Goal loops

Cursor has no `/goal` primitive; the human (or an automation) is the re-dispatcher:

```text
Use $planr-goal: <your goal>
Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract).
```

`$planr-loop` iterates inside the session, dispatching `/planr-worker` and `/planr-reviewer` per item. If the session ends before `planr plan audit <plan-id>` holds, send the same starter line in a fresh session — the map, the stored contract, and open reviews carry the run, not chat history.

## HTTP And Review Feedback

Planr V1 defaults to MCP stdio. Local HTTP/SSE is available through:

```bash
planr serve --port 7526
```

Cursor tasks can attach review feedback through either MCP stdio or the local HTTP API:

```bash
planr review annotate <item-id> --message "Cursor review note" --severity warning
planr review ingest <item-id> --from .planr/tmp/cursor-review.json
```

Review ingestion does not auto-close or auto-approve work.
