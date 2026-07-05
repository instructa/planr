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

## A Planr Command Appears To Hang

Planr bounds every database wait: `busy_timeout` is 5 seconds, and no command loops indefinitely — SQLite contention resolves or errors within that bound (a parallel first-pick storm is regression-tested). If a command still appears hung inside an agent-host tool call, the wait is almost certainly outside Planr: host tool harnesses that stop draining the child's stdout block the process on a full pipe, which looks exactly like a hang and works on retry. Kill and re-run the command; if it reproduces outside the host harness (plain terminal), capture a stack (`lldb -p <pid>` then `bt all`) and file it with the output — that would be a Planr bug we want.

## Database Or Import Issues

```bash
planr project show --json
planr import /path/to/repo --json
planr export --include-plans --include-logs --out planr-debug.json
```
