# Planr Operating Model

Planr coordinates coding-agent work through two durable surfaces:

- the Markdown plan package for product, build, architecture, verification, and narrative context;
- the SQLite map for item state, links, picks, reviews, logs, contexts, and closure.

The map is the source of truth for live state. Markdown explains why the work exists and what good completion means.

## Operator Start

Start every session by reading the current project and graph state:

```bash
planr project show --json
planr map show --json
planr map lane --critical
planr map pressure
```

If the repository has no Planr project yet:

```bash
planr project init "Project Name" --client all
planr doctor --client all
```

Use `--db <path>` for isolated runs and tests.

## Canonical Flow

The default product flow is:

```text
idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close
```

Use product and build plans for broad scope:

```bash
planr plan new "App idea" --platform web --ai --backend
planr plan refine <plan-id> --note "decision or assumption"
planr plan split <plan-id> --slice "MVP implementation"
planr plan check <build-plan-id>
planr map build --from <build-plan-id>
```

Product-plan task lists are candidates. Work becomes a live commitment only after it is in the map.

## Daily Agent Loop

Agents should work one map item at a time:

```bash
planr pick --json
planr trace item <item-id>
planr pick heartbeat <item-id>
```

For longer work, update runtime state instead of relying on chat history:

```bash
planr pick progress <item-id> --percent 50 --note "implementation done, tests running"
planr pick pause <item-id> --note "waiting for human decision"
planr pick resume <item-id>
```

If a previous worker disappears, inspect stale picks before taking over:

```bash
planr pick stale --older-than-seconds 900
planr pick stale --older-than-seconds 900 --release
planr recover sweep --older-than-seconds 900
planr recover sweep --older-than-seconds 900 --apply
```

Use `pick stale --release` for a targeted stale-claim reset. Use `recover sweep --apply` when you also want timed-out work and retryable failed work handled in the same explicit recovery pass.

Record discoveries that future work needs:

```bash
planr context add "decision or discovery" --item <item-id> --tag discovery
```

Record completion evidence before asking for review:

```bash
planr log add --item <item-id> \
  --summary "what changed" \
  --files path-a,path-b \
  --cmd "exact verification command"
```

Request and close review:

```bash
planr review request <item-id>
planr review annotate <item-id> --message "review note" --severity warning
planr review evidence <item-id> --pr-url https://example.invalid/pr/123
planr review ingest <item-id> --from .planr/tmp/review-feedback.json
planr review close <review-id> --verdict complete
planr close <item-id> --summary "Verified with evidence"
```

`review evidence` records Git branch, commit, dirty state, item-scoped changed-file provenance, and optional PR URL context without treating unrelated dirty files as proof. Review ingestion records hook-compatible JSON feedback as contexts and logs only. It does not close the review, approve the item, or unblock downstream work.

For a browser-based local review pass:

```bash
planr serve --port 8484
open http://127.0.0.1:8484/review
```

The review workspace shows review queues, linked plans, item evidence, diff-safe Git evidence, annotations, and approve/request-changes actions over the same local HTTP API.

Use approvals when a human decision must block closure:

```bash
planr approval request <item-id> --reason "release approval"
planr approval approve <item-id> --by "human reviewer"
```

Pending or denied approval blocks `planr close`; use `planr map preview --close <item-id>` to inspect the gate before mutating state.

If review finds issues:

```bash
planr review close <review-id> \
  --verdict not-complete \
  --findings "specific actionable finding"
planr review artifact <review-id>
planr map show --json
planr pick --json
```

Review findings create follow-up work and write a `.planr/reviews/*.review.md` artifact. Do not mark the original item complete by summarizing the finding away.

## Parent Gate Pattern

Model material changes as parent gates. The parent is not the work package; its linked children are.

```text
parent gate
`- implementation or test child
   `- review item linked to that child
      |- pass -> child can close -> parent can close
      `- findings -> fix item -> follow-up review -> ...
```

Rules:

- create one parent item for the change;
- use `planr item breakdown <parent-id> --into "Implement, Verify"` to create child work under the parent;
- request review on the implementation or test child after evidence exists;
- if review finds issues, let Planr create fix and follow-up review work from the review verdict;
- downstream top-level work should depend on the parent gate, not on the first implementation child.

This keeps later work blocked until review is actually clean.

## Notes, Contexts, Logs, And Stories

Use the smallest durable surface that fits the information:

- `planr log add`: proof that work happened, including files, commands, tests, review results, and handoff facts.
- `planr context add`: a project or item discovery that another future item may need.
- `planr note add`: a short task-local note when a human or agent needs nearby context.
- Story logs: longer narrative history when graph state and short contexts are too thin.

Story logs are narrative memory, not status authority. The map remains authoritative for state.

See [HANDOFFS_AND_STORIES.md](HANDOFFS_AND_STORIES.md) for file placement and contents.

## Recovery

After interruption, compaction, or agent handoff:

```bash
git status --short
planr project show --json
planr map show --json
planr map lane --critical
planr map pressure
```

Then inspect the current item:

```bash
planr trace item <item-id>
planr log list --item <item-id>
planr context list --item <item-id>
```

If ownership must be reset:

```bash
planr pick stale --older-than-seconds 900
planr pick release <item-id> --force
```

Use force only when the prior owner is gone or the operator intentionally resets the claim.

For broad interruption recovery, prefer the explicit sweeper:

```bash
planr recover sweep --older-than-seconds 900
planr recover sweep --older-than-seconds 900 --apply
```

The preview reports stale picks, timed-out work, retryable failures, retry delays, and manual pre/post conditions. The apply mode mutates only the listed recoverable work and records recovery events.

## Packages And Sharing

Use packages for local backups and reusable templates:

```bash
planr export --include-plans --include-logs --template-name "Backend slice" --tag api --out planr-package.json
planr import planr-package.json --preview
planr import planr-package.json --confirm
```

Import is preview-first. Confirmed import restores graph items, links, contexts, logs, plan file snapshots, and review artifacts into the current project. Encrypted sharing can wrap the JSON package with a local tool such as `age` or `gpg`; Planr does not require a hosted share service.

## Agent Prompts

Use prompt output when configuring agents without editing global config:

```bash
planr prompt cli --client codex
planr prompt mcp --client all
planr prompt http
```

Prompt commands print ready-to-use setup and operating instructions and report that global config was not edited.

## Completion Rule

Do not call a scope complete until all of these are true:

- required child and review items are closed;
- log evidence records exact files and commands;
- verification commands were actually run;
- review findings are closed or converted into follow-up work;
- `planr map show --json` has no in-scope blocker;
- the summary matches the map, logs, and review state.

For release-grade scopes, rerun the full verification ladder in [TESTING.md](TESTING.md).
