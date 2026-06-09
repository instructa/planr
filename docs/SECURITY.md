# Security And Privacy

- Planr is local-first and stores V1 data in the configured SQLite database and `.planr` files.
- No content telemetry is emitted by default.
- Shell commands are not run by Planr unless the user explicitly runs them outside Planr or records them as evidence.
- HTTP binds to `127.0.0.1`.
- Destructive operations require confirmation or preview flags.
- Logs and contexts are checked by `planr scrub` for common secret-looking patterns.
