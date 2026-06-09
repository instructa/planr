# Troubleshooting

## No Ready Items

```bash
planr map show --json
planr map pressure
planr trace item <item-id>
```

## MCP Client Cannot See Tools

```bash
planr doctor --client all
planr install codex --dry-run
planr install claude --dry-run
planr install cursor --dry-run
```

## Database Or Import Issues

```bash
planr project show --json
planr import /path/to/repo --json
planr export --include-plans --include-logs --out planr-debug.json
```
