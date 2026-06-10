# Cursor Integration

## Plugin

The repository carries a Cursor plugin manifest (`.cursor-plugin/plugin.json`) bundling the Planr skills. Marketplace listing is pending review; until it is listed, use MCP plus the CLI prompt below. See [Skills](SKILLS.md) for the skill workflow.

## MCP

```bash
planr install cursor --dry-run
planr install cursor
planr doctor --client cursor
```

Dry-run prints `.cursor/mcp.json` content. The non-dry command writes the project-scoped config.

Planr V1 defaults to MCP stdio. Local HTTP/SSE is available through:

```bash
planr serve --port 7526
```

Cursor tasks can attach review feedback through either MCP stdio or the local HTTP API:

```bash
planr review annotate <item-id> --message "Cursor review note" --severity warning
planr review ingest <item-id> --from .planr/tmp/cursor-review.json
```

The project-scoped `.cursor/mcp.json` is the only file written by `planr install cursor`. Review ingestion does not auto-close or auto-approve work.
