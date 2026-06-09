# Planr Real-World Flow

```bash
planr project init "Example Product" --client all
planr plan new "Inventory API" --platform api --backend
planr plan split <plan-id> --slice "MVP backend"
planr map build --from <build-plan-id>
planr pick
planr log add --item <item-id> --summary "Implemented schema and routes" --files src/api.rs --cmd "cargo test"
planr review request <item-id>
planr review close <review-id> --verdict complete
planr close <item-id> --summary "Verified"
planr search "schema"
planr doctor --client all
planr install codex --dry-run
planr install claude --dry-run
planr install cursor --dry-run
```

The graph state lives in SQLite. Plans remain readable Markdown under `.planr/plans/`. The same engine is available through CLI, MCP stdio, and the local HTTP/SSE server.
