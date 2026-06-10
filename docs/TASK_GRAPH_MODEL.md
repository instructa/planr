# Planr Task Graph Model

Planr's map is a local dependency-aware graph for coding-agent work. It exists so agents can coordinate without relying on ad hoc chat history.

## Core Objects

- Project: repository-level Planr workspace.
- Plan: Markdown product or build package.
- Item: one unit of live work in the map.
- Link: relationship between items.
- Pick: atomic claim of a ready item by one worker session.
- Log: durable proof of implementation, verification, review, or handoff.
- Review: an item that blocks target closure until it is closed.
- Context: searchable discovery or decision.
- Recovery: explicit preview/apply operation for stale, timed-out, or retryable work.

## State Authority

The SQLite database is authoritative for:

- item status;
- links and readiness;
- picks and ownership;
- reviews and closure gates;
- contexts and logs;
- artifacts and events as they become public product surfaces.

Markdown plans are authoritative for scope and acceptance criteria. They do not override map state.

## Item Readiness

Readiness is derived from state and links:

- an item may become ready when its blocking upstream items are closed;
- picked items are owned by one worker session;
- closed items can unlock downstream items;
- open review items block target closure;
- cancelled items should not silently unlock unrelated work.

Use:

```bash
planr map show --json
planr map lane --critical
planr map pressure
```

## Links

Create explicit order:

```bash
planr item create "Design API" --description "Define endpoints and data ownership."
planr item create "Implement API" --description "Build endpoints after design is closed."
planr link add <design-item> <implementation-item> --type blocks
```

Use `blocks` for hard ordering. Use softer link types only when the product contract for that type is documented and tested.

## Picking

Picking is the concurrency boundary:

```bash
planr pick --json
```

One ready item should be picked by one worker. Picked work records `worker_id`, `pick_token`, `picked_at`, and `last_heartbeat_at`. Worker identity is stable per client, host, and user, so heartbeats keep working across the many short-lived processes of an agent session. Parallel workers on the same machine must set `PLANR_SESSION_ID` to distinct values. Active agents should keep the claim fresh and make progress visible:

```bash
planr pick heartbeat [item-id]
planr pick progress <item-id> --percent 42 --note "running focused tests"
planr pick pause <item-id> --note "waiting for review input"
planr pick resume <item-id>
```

If two workers race, the database must allow only one winner. If the owner disappears, inspect stale claims and reset intentionally:

```bash
planr pick stale --older-than-seconds 900
planr pick stale --older-than-seconds 900 --release
planr recover sweep --older-than-seconds 900
planr recover sweep --older-than-seconds 900 --apply
planr pick release <item-id> --force
```

Use forced release only when the operator intentionally transfers ownership. Use `recover sweep --apply` for the broader recovery path that also requeues timed-out picked work and retryable failed work.

## Approvals

Approvals are explicit human gates on an item:

```bash
planr approval request <item-id> --reason "needs release approval"
planr approval deny <item-id> --by "qa" --comment "missing evidence"
planr approval approve <item-id> --by "qa" --comment "evidence accepted"
planr approval list --open
```

An item with `requested` or `denied` approval status cannot close. `map preview --close <item-id>` reports whether approval blocks closure before the mutation is attempted.

## Reviews And Fix Chains

Request review after evidence exists:

```bash
planr review request <item-id>
```

A clean review can close:

```bash
planr review close <review-id> --verdict complete
```

Findings create follow-up work:

```bash
planr review close <review-id> \
  --verdict not-complete \
  --findings "specific actionable finding"
```

The target item may close only when required review items are closed.

## Evidence

Logs make closure auditable:

```bash
planr log add --item <item-id> \
  --summary "Implemented parser hardening" \
  --files src/parser.rs,tests/parser.rs \
  --cmd "cargo test parser"
```

Use multiple `--cmd` or `--tests` values when more than one verification command matters.

## Graph Inspection

Use critical lane for ordering risk:

```bash
planr map lane --critical
```

Use pressure for bottlenecks:

```bash
planr map pressure
```

Use trace for handoff:

```bash
planr trace item <item-id>
```

The trace should be enough for a new agent to recover the current item, linked logs, and linked blockers.

Use status, lookahead, and close preview before sequencing or closure:

```bash
planr map status
planr map lookahead <item-id>
planr map preview --close <item-id>
```

These commands expose readiness, downstream unlocks, closure blockers, approval requirements, open reviews, and manual conditions before mutating graph state.

## Review Evidence

Review evidence is item-scoped proof, not a dump of the whole repository:

```bash
planr review evidence <item-id>
planr review evidence <item-id> --pr-url https://example.invalid/pr/123
```

Planr reports Git branch, commit, dirty state, files named by item logs/artifacts, unrelated dirty files, and optional PR URL context. It does not inline source file content by default.

## Packages

Packages preserve reusable graph context outside the live database:

```bash
planr export --include-plans --include-logs --template-name "Release checklist" --tag release --out planr-package.json
planr import planr-package.json --preview
planr import planr-package.json --confirm
```

Package import is preview-first and confirmed explicitly. Imported packages restore items, links, contexts, logs, plan file snapshots, and review artifacts into the current map.

## Graph Adaptation

Planr supports graph adaptation without relying on chat-only replanning:

- use `item breakdown` for decomposition;
- use `item insert --preview` before rewiring linked work and `--confirm` to apply;
- use `item amend` for future-work context;
- use `item replan --preview` before replacing pending child work and `--confirm` to apply;
- use `link add` and `link remove` for dependency changes;
- use `map preview --close`, `map unlocks`, `map lookahead`, and `map status` before closing or sequencing work;
- use `item cancel --preview` before cancellation;
- use `trace item` and logs for recovery.
