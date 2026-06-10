# Claude Code Integration

## Plugin (preferred for skills)

The Planr repository is a Claude Code plugin. Install it to get the skills (namespaced as `/planr:planr`, `/planr:planr-loop`, ...) plus the `planr-worker` and `planr-reviewer` subagents:

```text
/plugin marketplace add instructa/planr
/plugin install planr@planr
```

See [Skills](SKILLS.md) for the skill workflow. For autonomous goal runs with `/goal` or `/loop` on top of Planr state, see [Long-Running Goals](GOALS.md).

## MCP

```bash
planr install claude --dry-run
planr install claude
planr doctor --client claude
```

Dry-run prints both project-scope `.mcp.json` content and the user-scope CLI form. The non-dry command writes only this repository's `.mcp.json`.

Claude Code should treat Planr map state as authoritative and use Markdown plans as context.

For repo-local review feedback, write JSON to a file and ingest it:

```bash
planr review ingest <item-id> --from .planr/tmp/claude-review.json
planr review artifact <review-item-id>
```

Planr does not install shell hooks or edit global Claude Code configuration. The review item remains open until `planr review close` records the final verdict.
