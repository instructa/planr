# API And Data Model

## Storage Locations

- SQLite: authoritative map graph state, picks, contexts, logs, runs, events, search indexes.
- `.planr/project/*.md`: durable project context pack.
- `.planr/plans/product/<slug>/`: product specification packages.
- `.planr/plans/build/*.plan.md`: Markdown implementation plans.
- `.planr/reviews/*.review.md`: optional saved review artifacts.
- Git: source diffs, commits, branches, and worktrees.

## Core Tables

### projects

```text
id TEXT PRIMARY KEY
name TEXT NOT NULL
root_path TEXT NOT NULL
description TEXT
status TEXT NOT NULL
metadata JSON
created_at DATETIME
updated_at DATETIME
```

### items

```text
id TEXT PRIMARY KEY
project_id TEXT NOT NULL
parent_item_id TEXT
title TEXT NOT NULL
description TEXT
status TEXT NOT NULL
work_type TEXT NOT NULL
priority INTEGER
worker_id TEXT
plan_path TEXT
pick_token TEXT
picked_at DATETIME
last_heartbeat_at DATETIME
progress_percent INTEGER
progress_note TEXT
paused_at DATETIME
timeout_seconds INTEGER
max_retries INTEGER
retry_count INTEGER
retry_backoff TEXT
retry_delay_ms INTEGER
pre_condition TEXT
post_condition TEXT
approval_status TEXT
approval_requested_at DATETIME
approved_by TEXT
approval_comment TEXT
started_at DATETIME
completed_at DATETIME
result JSON
error TEXT
metadata JSON
created_at DATETIME
updated_at DATETIME
```

Item work types:

- generic
- research
- plan
- code
- review
- fix
- test
- shell
- release

Item statuses:

- pending
- ready
- picked
- running
- in_review
- blocked
- closed
- closed_partial
- failed
- cancelled

### links

```text
id INTEGER PRIMARY KEY
from_item TEXT NOT NULL
to_item TEXT NOT NULL
kind TEXT NOT NULL
condition TEXT NOT NULL
metadata JSON
UNIQUE(from_item, to_item, kind)
```

Link kinds:

- blocks: upstream must complete before downstream is ready.
- feeds_into: upstream result is included in downstream handoff.
- reviews: review item blocks parent/target closure.
- relates_to: non-blocking context relationship.

### plans

```text
id TEXT PRIMARY KEY
project_id TEXT NOT NULL
stage TEXT NOT NULL
path TEXT NOT NULL
title TEXT NOT NULL
slug TEXT NOT NULL
package_manifest JSON
frontmatter JSON
parse_status TEXT NOT NULL
content_hash TEXT NOT NULL
created_at DATETIME
updated_at DATETIME
```

Plan stages:

- product
- build
- review

### source_links

```text
id INTEGER PRIMARY KEY
source_type TEXT NOT NULL
source_id TEXT NOT NULL
item_id TEXT NOT NULL
section_id TEXT
relationship TEXT NOT NULL
```

Relationships:

- scopes
- implements
- verifies
- reviews
- references

### contexts

```text
id TEXT PRIMARY KEY
project_id TEXT NOT NULL
item_id TEXT
worker_id TEXT
kind TEXT NOT NULL
content TEXT NOT NULL
tags JSON
created_at DATETIME
```

Kinds:

- discovery
- decision
- constraint
- pattern
- blocker
- bug
- risk

### runs

```text
id TEXT PRIMARY KEY
project_id TEXT NOT NULL
item_id TEXT NOT NULL
worker_id TEXT NOT NULL
client TEXT NOT NULL
profile TEXT
command TEXT
cwd TEXT
worktree_path TEXT
status TEXT NOT NULL
started_at DATETIME
ended_at DATETIME
exit_code INTEGER
metadata JSON
```

Clients:

- codex
- claude-code
- cursor
- generic-mcp
- human
- ci

### logs

```text
id TEXT PRIMARY KEY
project_id TEXT NOT NULL
item_id TEXT NOT NULL
run_id TEXT
kind TEXT NOT NULL
summary TEXT NOT NULL
files JSON
commands JSON
tests JSON
review_findings JSON
blocked_or_unverified JSON
created_at DATETIME
```

Log kinds:

- completion
- review
- failure
- handoff
- verification

### artifacts

```text
id TEXT PRIMARY KEY
project_id TEXT NOT NULL
item_id TEXT
name TEXT NOT NULL
kind TEXT
path TEXT
content TEXT
mime_type TEXT
size_bytes INTEGER
metadata JSON
created_at DATETIME
```

### events

```text
id INTEGER PRIMARY KEY
project_id TEXT
item_id TEXT
worker_id TEXT
event_type TEXT NOT NULL
payload JSON
timestamp DATETIME
```

## Plan Package Contract

Product plan packages use this structure:

```text
.planr/plans/product/<slug>/
  README.md
  PRODUCT_SPEC.md
  UX_FLOWS.md
  DESIGN_SYSTEM_SPEC.md
  TECH_ARCHITECTURE.md
  ADRS.md
  AI_SPEC.md
  SAFETY_PRIVACY_SECURITY.md
  API_AND_DATA_MODEL.md
  CLIENT_IMPLEMENTATION_SPEC.md
  BACKEND_IMPLEMENTATION_SPEC.md
  ANALYTICS_OBSERVABILITY_SPEC.md
  QA_ACCEPTANCE_TESTS.md
  RELEASE_READINESS.md
  TASKS.md
  REFERENCES.md
```

- REQ-API-001: Plan packages must include a manifest with generated date, source prompt, assumptions, and included documents.
- REQ-API-002: Plan work lists are candidate work, not live map items until accepted.

## Build Plan Contract

Plan files use this minimum shape:

```markdown
---
name: short-name
overview: One paragraph.
todos:
  - id: phase-1
    content: Summary item.
    status: pending
isProject: false
---

# Plan Title

## Scope Decision
## Ownership Target
## Existing Leverage
## Phase 1: ...
## Out Of Scope
## Verification
## Acceptance Criteria
```

- REQ-API-010: Plan parsing must preserve unknown frontmatter fields.
- REQ-API-011: Plan parsing failure must not delete or rewrite the original Markdown.
- REQ-API-012: Closure status must not be inferred solely from Markdown checkboxes.

## CLI API

### Project

```bash
planr project init [--client codex|claude|cursor|all] [--force]
planr project show [--json]
planr project list [--json]
planr doctor [--client codex|claude|cursor|all]
planr install codex|claude|cursor [--dry-run]
planr prompt cli|mcp|http [--client codex|claude|cursor|all]
planr mcp
planr serve --port 8484
planr import <file> [--preview] [--confirm]
planr export --out planr.json [--include-plans] [--include-logs] [--template-name "..."] [--tag tag]
```

### Plan

```bash
planr plan new "App idea" [--platform web] [--ai] [--backend]
planr plan refine <plan-id>
planr plan split <plan-id> --slice "MVP backend"
planr plan check <plan-id>
planr plan show <plan-id>
planr plan archive <plan-id>
```

### Map

```bash
planr map show
planr map build --from <plan-id>
planr map status
planr map preview --close <item-id>
planr map unlocks <item-id>
planr map lookahead [--from <item-id>]
planr item create "title" --description "..." [--after item-id] [--timeout-seconds N] [--max-retries N] [--retry-backoff fixed|exponential] [--retry-delay-ms N] [--pre "..."] [--post "..."]
planr item breakdown <item-id> --into "A, B, C"
planr item insert "title" --description "..." --after <item-id> [--before <item-id>] [--preview|--confirm]
planr item amend <item-id> --note "..."
planr item replan <parent-id> --into "A, B, C" [--preview|--confirm]
planr link add <from-item> <to-item> --type blocks
planr pick
planr pick release <item-id> [--force]
planr pick heartbeat [item-id]
planr pick progress <item-id> --percent 0..100 [--note "..."]
planr pick pause <item-id> [--note "..."]
planr pick resume <item-id>
planr pick stale [--older-than-seconds 900] [--release]
planr recover sweep [--older-than-seconds 900] [--apply]
planr approval request <item-id> [--reason "..."]
planr approval approve <item-id> --by "name" [--comment "..."]
planr approval deny <item-id> --by "name" [--comment "..."]
planr approval list [--open]
planr artifact add "name" [--item <item-id>] [--kind evidence] [--path file] [--content "..."]
planr artifact show <artifact-id>
planr artifact list [--item <item-id>]
planr event list [--item <item-id>] [--limit 50]
planr debug bundle [--item <item-id>] --preview
planr log add --item <item-id> --summary "..." [--files a,b] [--cmd "..."]
planr review request <item-id>
planr review annotate <item-id> --message "..." [--severity info|warning|blocking] [--file path] [--line N] [--author "..."]
planr review ingest <item-id> (--from feedback.json|--stdin)
planr review artifact <review-item-id> [--out path]
planr review evidence <item-id> [--pr-url https://...]
planr review close <review-item-id> --verdict complete|not-complete|unclear
planr close [item-id] --summary "..." [--next]
planr map lane --critical
planr map pressure
```

## MCP Tools

- `planr_project_show`
- `planr_map_show`
- `planr_map_status`
- `planr_map_preview`
- `planr_map_unlocks`
- `planr_map_lookahead`
- `planr_plan_create`
- `planr_plan_refine`
- `planr_plan_split`
- `planr_plan_check`
- `planr_plan_link`
- `planr_map_build`
- `planr_item_create`
- `planr_item_breakdown`
- `planr_item_insert`
- `planr_item_amend`
- `planr_item_replan`
- `planr_pick_item`
- `planr_pick_heartbeat`
- `planr_pick_progress`
- `planr_pick_pause`
- `planr_pick_resume`
- `planr_pick_stale`
- `planr_recover_sweep`
- `planr_approval_request`
- `planr_approval_approve`
- `planr_approval_deny`
- `planr_approval_list`
- `planr_artifact_add`
- `planr_artifact_list`
- `planr_artifact_show`
- `planr_event_list`
- `planr_debug_bundle`
- `planr_log_add`
- `planr_review_annotate`
- `planr_review_ingest`
- `planr_review_artifact`
- `planr_review_evidence`
- `planr_review_close`
- `planr_close_item`
- `planr_context_create`
- `planr_search`
- `planr_log_read`

## MCP Resources

- `planr://project/context`
- `planr://project/map`
- `planr://item/{id}`
- `planr://plan/{id}`
- `planr://log/{id}`

## MCP Prompts

- `planr-plan`
- `planr-work`
- `planr-review`
- `planr-map`
- `planr-summary`

## HTTP API

HTTP is optional in V1 and localhost-only by default.

```text
GET    /review
GET    /v1/review-workspace
GET    /v1/projects
POST   /v1/projects
GET    /v1/projects/:id/map
GET    /v1/projects/:id/items
POST   /v1/projects/:id/items
POST   /v1/pick
POST   /v1/items/:id/heartbeat
POST   /v1/items/:id/progress
POST   /v1/items/:id/pause
POST   /v1/items/:id/resume
POST   /v1/items/:id/approval/request
POST   /v1/items/:id/approval/approve
POST   /v1/items/:id/approval/deny
GET    /v1/approvals?open=true
POST   /v1/artifacts
GET    /v1/artifacts
GET    /v1/artifacts/:id
GET    /v1/events
GET    /v1/debug/bundle
POST   /v1/items/:id/log
POST   /v1/items/:id/close
POST   /v1/items/:id/reviews
POST   /v1/items/:id/review-annotations
GET    /v1/items/:id/review-evidence
POST   /v1/items/:id/review-evidence
POST   /v1/items/:id/review-feedback
POST   /v1/reviews/:id/close
POST   /v1/reviews/:id/artifact
GET    /v1/reviews/:id/artifact
POST   /v1/contexts
GET    /v1/search?q=...
GET    /v1/events/stream
```

## Error Model

```json
{
  "error": {
    "code": "not_found",
    "message": "item not found",
    "details": {}
  }
}
```

Required codes:

- bad_request
- not_found
- conflict
- invalid_transition
- locked
- parse_error
- unauthorized_remote
- internal_error

## Retention

- Local database persists until user deletes it.
- Plan files persist in Git/repo unless archived or deleted.
- Log persists locally by default.
- Full transcript retention is off by default.
