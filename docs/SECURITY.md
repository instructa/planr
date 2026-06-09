# Security And Privacy

- Planr is local-first and stores V1 data in the configured SQLite database and `.planr` files.
- No content telemetry is emitted by default.
- Shell commands are not run by Planr unless the user explicitly runs them outside Planr or records them as evidence.
- HTTP binds to `127.0.0.1`.
- Destructive operations require confirmation or preview flags.
- Logs, contexts, and inline artifact content are checked by `planr scrub` for common secret-looking patterns. `planr scrub --confirm` rewrites flagged values in place with `[REDACTED]` markers, updates the search index, and records a `secret_scrubbed` event per rewritten row.
