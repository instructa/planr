# Recovery And Verification

Load this only for an interrupted item or the live-verification stage.

Recovery:

```bash
planr plan audit <plan-id> --json
planr trace item <item-id> --json
planr log list --item <item-id> --json
planr context list --item <item-id> --json
```

Use `planr pick stale --older-than-seconds 900` before releasing abandoned ownership. Pause a legitimate wait with `planr pick pause`; resume it with `planr pick resume`.

Verification:

```bash
planr log add --item <item-id> --kind verification \
  --summary "verified <flow>: <observed outcome>" \
  --cmd "<exact replayable command>"
```

Use browser automation for web, the built binary for CLI, real requests for API/backend, and simulator launch plus exercised flow for iOS. If tooling is unavailable, store blocker context, request approval, and pause.
