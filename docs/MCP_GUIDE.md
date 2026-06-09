# MCP Guide

Planr exposes one stdio MCP server:

```bash
planr mcp
```

Core tools include project/map reads, map status/preview/unlocks/lookahead, plan creation/refinement/splitting, map build, item create/breakdown/insert/amend/replan, pick, runtime heartbeat/progress/pause/resume/stale inspection, recovery sweep, approval request/approve/deny/list, artifact add/list/show, event list, debug bundle preview, log, review annotate/ingest/artifact/evidence/close, close, context create, search, and log read.

Review feedback tools:

- `planr_review_annotate`: add item-linked review feedback with severity, optional file, line, and author.
- `planr_review_ingest`: ingest hook-compatible JSON feedback without auto-closing or auto-approving work.
- `planr_review_artifact`: write a privacy-minimized `.planr/reviews/*.review.md` artifact.
- `planr_review_evidence`: return scoped Git/PR review evidence without source contents.
- `planr_review_close`: close a review item, write a review artifact, and create fix/follow-up review work when the verdict is not clean.

Resources:

- `planr://project/map`
- `planr://project/context`
- `planr://item/{id}`
- `planr://plan/{id}`
- `planr://log/{id}`

Prompts:

- `planr-plan`
- `planr-work`
- `planr-review`
- `planr-map`
- `planr-summary`

Use `planr install <client> --dry-run` to print project-scoped config.

The stable V1 contract and checked fixture live in:

- `docs/MCP_CONTRACT.md`
- `docs/fixtures/mcp-contract.json`
