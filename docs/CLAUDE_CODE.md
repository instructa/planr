# Claude Code Integration

## Plugin (preferred for skills)

The Planr repository is a Claude Code plugin. Install it to get the skills (namespaced as `/planr:planr`, `/planr:planr-loop`, ...) plus the `planr-worker` and `planr-reviewer` subagents:

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

See [Skills](SKILLS.md) for the skill workflow. For autonomous goal runs with `/goal` or `/loop` on top of Planr state, see [Long-Running Goals](GOALS.md).

## Long-Running Goals With `/goal`

Claude Code `/goal` drives autonomous Planr runs the same way Codex does: `/goal` supplies continuation pressure, Planr supplies durable state, evidence, reviews, and recovery. Run the driver session on your strongest model (`/model fable`, `/effort high`), prep once, then start:

```text
/planr:planr-goal <your goal>
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted. You are operating autonomously: the user is not watching, so never end a turn on a plan, a question, or a promise — proceed until the contract holds or you are blocked on input only the user can provide.
```

The plugin registers the `planr-worker` and `planr-reviewer` subagents automatically. The worker pins a cheaper tier in its frontmatter; the reviewer deliberately inherits the driver's model:

```yaml
# planr-worker.md frontmatter
model: opus      # alias tracks the current generation; budget alternative: sonnet
effort: medium
```

Verify the pin once: `CLAUDE_CODE_SUBAGENT_MODEL` must be unset (it silently overrides all subagent frontmatter), then dispatch the worker on a trivial item and confirm the subagent's messages in `~/.claude/projects/<project>/*.jsonl` carry the worker model. Full workflow, recovery, and the tiering rationale: [Long-Running Goals](GOALS.md).

## MCP

```bash
planr install claude --dry-run
planr install claude
planr doctor --client claude
```

Dry-run prints both project-scope `.mcp.json` content and the optional user-scope CLI form. The non-dry command writes this repository's `.mcp.json`, standalone worker/reviewer roles, and additive fail-open session hooks. Workflow skills and plugin agents come from the Claude Code plugin. Use `--no-mcp` to omit `.mcp.json`, or `--no-hooks` to omit hook reconciliation.

Claude Code should treat Planr map state as authoritative and use Markdown plans as context.

For repo-local review feedback, write JSON to a file and ingest it:

```bash
planr review ingest <item-id> --from .planr/tmp/claude-review.json
planr review artifact <review-item-id>
```

Planr does not edit global Claude Code configuration. Project hooks live in `.claude/settings.json`, preserve foreign entries, and can be omitted with `--no-hooks`. The review item remains open until `planr review close` records the final verdict.
