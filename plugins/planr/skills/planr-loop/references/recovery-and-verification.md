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
planr evidence verify --scope plan --id <plan-id> --json
```

The verify result owns plan coverage and settlement. Use criterion-scoped coverage or explain only to diagnose an explicit gap, with a real criterion id rather than a requirement id. A verifier or environment failure stops without an automatic retry.

The Evidence broker executes only the sealed run index from its own readiness stage. A repository-authored obligation, declarative index, guessed digest path, or remembered run filename is not executable Evidence input.

Use a capability whose declared observations cover the criterion: browser automation for rendered web behavior, the built binary for CLI, real requests for API/backend, and simulator launch plus exercised flow for iOS. If tooling is unavailable, preserve the typed blocker, request approval when appropriate, and pause. Do not replace it with a weaker method that observes different facts.
