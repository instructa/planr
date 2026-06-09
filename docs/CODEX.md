# Codex Integration

```bash
planr install codex --dry-run
planr doctor --client codex
```

The dry-run prints the MCP server snippet for `planr mcp`. Verify the client-side registration with the Codex CLI command shown by your local Codex installation.

Codex should use the same public flow as every other client:

```text
map -> pick -> work -> log -> review -> close
```

Review hooks can feed Planr without changing global Codex settings:

```bash
codex review --json > .planr/tmp/codex-review.json
planr review ingest <item-id> --from .planr/tmp/codex-review.json
planr review annotate <item-id> --message "Needs regression coverage" --severity blocking
```

Ingested feedback is evidence only. A reviewer or agent must still close the review item explicitly with `planr review close`.
