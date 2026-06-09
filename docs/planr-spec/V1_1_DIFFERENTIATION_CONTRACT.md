# V1.1 Differentiation Contract

Generated: 2026-06-09

## Purpose

This contract defines the Planr-owned V1.1 acceptance baseline. It turns the remaining product gaps into implementation-ready requirements without relying on external product names, copied command vocabulary, or hidden context.

V1.1 is complete only when Planr can prove these capabilities from its own CLI, MCP, HTTP, local review workspace, docs, tests, and real consumer usage.

## Product Outcome

Planr V1.1 must feel like one coherent product:

```text
idea -> product plan -> build plan -> map -> pick -> work -> log -> review -> recover -> close
```

The map remains the source of truth for execution state. Markdown plans remain the source of rich product and implementation context. Review and recovery must be evidence-backed, not optimistic.

## Capability Requirements

### Graph Intelligence

- REQ-V11-GRAPH-001: `planr map lane --critical` must compute a real critical path across blocking graph edges, not just sort open items by priority.
- REQ-V11-GRAPH-002: `planr map pressure` must report transitive downstream impact, direct blockers, and why each item matters.
- REQ-V11-GRAPH-003: Map reads must detect and report cycles with enough edge detail for a user or agent to repair the graph.
- REQ-V11-GRAPH-004: Unlock and lookahead views must explain what becomes ready after closure, what remains blocked, and which blockers are still active.
- REQ-V11-GRAPH-005: Mutation previews must share one effect engine for close, cancel, insert, replan, and dependency rewires.
- REQ-V11-GRAPH-006: CLI, MCP, HTTP, and JSON responses must use the same graph-analysis results.

Acceptance:

- A branched fixture with deep dependencies returns the mathematically longest open blocking path.
- A transitive bottleneck ranks above a shallow item that only blocks one direct child.
- A cyclic graph is refused or diagnosed before readiness or critical-path output becomes misleading.

### Automatic Pick Recall

- REQ-V11-RECALL-001: `planr pick` and `planr_pick_item` must return a compact task-start package.
- REQ-V11-RECALL-002: The package must include ranked relevant project contexts, upstream logs or handoffs, linked plan references, active blockers or unlocks, review/fix history, and deeper-read commands.
- REQ-V11-RECALL-003: Recall ranking must search item title, description, linked plan metadata, contexts, logs, and review summaries.
- REQ-V11-RECALL-004: Pick recall must be size-bounded and avoid source file content, prompt transcripts, secret-looking values, and large artifacts by default.
- REQ-V11-RECALL-005: Possible file conflicts must be surfaced from recent logs, artifacts, and active picked/running work when enough path evidence exists.

Acceptance:

- A task with relevant historical decisions receives those decisions without the user running a separate search.
- Irrelevant contexts are omitted or ranked below relevant ones.
- The response remains compact enough for an agent handoff and includes deeper-read commands for full detail.

### Recovery Automation

- REQ-V11-RECOVERY-001: Planr must provide a sweeper command and API that inspect interrupted work without silently mutating by default.
- REQ-V11-RECOVERY-002: Timeouts must be stored on items and evaluated consistently for CLI, MCP, and HTTP flows.
- REQ-V11-RECOVERY-003: Retry policy must support max attempts, retry count, fixed or exponential delay, and clear terminal state when retries are exhausted.
- REQ-V11-RECOVERY-004: Stale picks must be reclaimable only through explicit recovery action or a documented sweeper mode.
- REQ-V11-RECOVERY-005: Item preconditions and postconditions must be visible in pick, trace, review, and close-preview outputs.
- REQ-V11-RECOVERY-006: Close preview must show unsatisfied conditions, missing evidence, open reviews, open child work, and approval blockers in one gate report.

Acceptance:

- An interrupted picked item is diagnosed, optionally released, and then picked by a later worker without duplicate ownership.
- A failed retryable item returns to ready only after the documented delay.
- A postcondition appears in review and close-preview output before the item closes.

### Local Browser Review Workspace

- REQ-V11-REVIEW-UI-001: Planr must ship a local browser workspace reachable from the local HTTP server or an explicit CLI command.
- REQ-V11-REVIEW-UI-002: The workspace must support plan/package review, plan revision diff, item review detail, inline annotations, and approve/request-changes feedback.
- REQ-V11-REVIEW-UI-003: When Git evidence is available, the workspace must show scoped file changes or a clear explanation of missing diff context.
- REQ-V11-REVIEW-UI-004: Feedback must write Planr review annotations, review artifacts, and map-native fix/follow-up review items through existing review rules.
- REQ-V11-REVIEW-UI-005: The workspace must remain local-first and must not require a hosted account or network service.

Acceptance:

- A reviewer can open a local URL, annotate a plan or item, request changes, and see the map create follow-up work.
- A clean review can write an artifact and close the review item through Planr state.
- Browser smoke tests prove the workspace renders and can exercise the annotation flow.

### Scoped Git And PR Review

- REQ-V11-GIT-001: Planr must detect the current Git worktree and record branch, commit, dirty state, and changed-file scope in review evidence.
- REQ-V11-GIT-002: Review flows must distinguish agent-owned evidence from unrelated dirty files.
- REQ-V11-GIT-003: Item logs and artifacts must be able to reference changed files and line-level findings without storing full source contents by default.
- REQ-V11-GIT-004: Optional PR URL review may be implemented when a provider can be queried safely; otherwise V1.1 must clearly document local-only behavior and future PR support.
- REQ-V11-GIT-005: Review output must tie findings to item id, file path, optional line, evidence source, and next action.

Acceptance:

- A dirty worktree with unrelated files does not let an agent claim broad ownership.
- Review artifacts show which files were considered and which were excluded.
- File and line annotations survive export/import.

### Distribution And Client Setup

- REQ-V11-DIST-001: README and install docs must make GitHub Release curl installation the normal user path.
- REQ-V11-DIST-002: Homebrew instructions must be present as soon as a tap or formula path exists; until then docs must state the release condition clearly.
- REQ-V11-DIST-003: Cargo and source build instructions must be maintainer/developer paths, not the primary user install story.
- REQ-V11-DIST-004: `planr install` or equivalent setup commands must cover Codex, Claude Code, Cursor, and generic MCP without editing global configuration unexpectedly.
- REQ-V11-DIST-005: `planr prompt` or an equivalent command must expose ready-to-use CLI, MCP, and HTTP instructions.
- REQ-V11-DIST-006: Release artifacts must have checksums and documented verification steps.

Acceptance:

- A fresh machine can install from release assets using only documented commands.
- A project can configure each supported client from Planr docs or CLI output.
- Package dry-runs show only intentional files.

### Templates And Review Packages

- REQ-V11-TEMPLATE-001: Export/import must support map items, links, plans, contexts, logs, review artifacts, and metadata.
- REQ-V11-TEMPLATE-002: Templates must include compatibility metadata, Planr version, creation timestamp, source project name, and optional tags.
- REQ-V11-TEMPLATE-003: Import must preview what will be created or skipped before mutating existing projects.
- REQ-V11-TEMPLATE-004: Review packages must preserve annotations, findings, artifacts, and file references.
- REQ-V11-TEMPLATE-005: Encrypted local bundle sharing may be implemented without hosted infrastructure; if not implemented in V1.1, docs must capture the accepted local-first format and explicit future scope.

Acceptance:

- A proven decomposition can be exported, imported into a fresh project, and picked without losing dependencies.
- Review annotations survive package export/import.
- Imports are idempotent or explain conflicts before mutation.

### Documentation And Public Copy

- REQ-V11-DOCS-001: README, CLI reference, MCP contract, operating model, task graph model, install docs, testing docs, and spec files must match implemented V1.1 behavior.
- REQ-V11-DOCS-002: `docs/planr-spec.zip` must be regenerated after spec changes.
- REQ-V11-DOCS-003: Docs must use Planr vocabulary: plan, map, item, pick, log, review, context, recovery, package.
- REQ-V11-DOCS-004: Repo-visible docs and public copy must not mention local comparison products or imply Planr is a fork of another task graph or review tool.
- REQ-V11-DOCS-005: New commands and APIs must have examples and JSON contract notes where agents rely on them.

Acceptance:

- Docs, fixtures, CLI help, MCP tools, and tests do not drift.
- A forbidden-reference scrub over public repo paths returns no matches for comparison-product names.

## End-To-End Verification Contract

V1.1 final verification must run from the current worktree and a fresh consumer project at `~/projects/planr-test`.

Required repository checks:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `scripts/build-release.sh`
- checksum verification for release assets and packaged files
- `npm pack --dry-run` when the npm wrapper remains present
- `scripts/ci-local.sh`
- MCP stdio contract smoke
- HTTP/SSE smoke
- local browser review workspace smoke
- forbidden-reference scrub over public repo paths

Required consumer checks in `~/projects/planr-test`:

- install or invoke the built Planr binary without relying on source-tree paths except for the selected test binary
- initialize a project
- create a product plan or build plan
- build map items
- pick one item
- log files, commands, and tests
- request review
- annotate or ingest review feedback
- close review and target item according to gate rules
- export and import a package/template
- run documented examples that are meant to work for users

The final V1.1 review must treat missing, stale, narrow, or indirect evidence as incomplete.

## Out Of Scope For V1.1 Unless Explicitly Added

- Hosted accounts, billing, hosted sync, or team permission models.
- Uploading private plans, source files, prompts, responses, or review content to a hosted service by default.
- Full project-management SaaS workflows unrelated to coding-agent execution.
- Silent global agent-client configuration edits.
- Claiming PR-provider support that was not verified with a real provider or documented local fallback.
