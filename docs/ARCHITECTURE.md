# Planr Architecture

Planr V1 is a single Rust binary with explicit module ownership. The crate stays small enough that a Cargo workspace would add more process overhead than value today, and there is only one deployable: the `planr` CLI. The source tree is split by ownership boundary inside that crate instead of using a premature workspace. Shared mutations that must behave identically across CLI, MCP, and HTTP live in one place (`src/app/application.rs`) so the three surfaces cannot drift.

## Repository Layout

- `src/`: the Rust CLI (module ownership below).
- `tests/e2e.rs`: real CLI, MCP, HTTP, import, review-gate, run-log, and concurrent-pick tests.
- `plugins/planr/`: the installable plugin payload — all nine skills, the worker and reviewer subagent roles, and the per-host plugin manifests.
- `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`: marketplace manifests pointing Codex and Claude Code at `plugins/planr`.
- `docs/`: user and contributor guides; `docs/planr-spec/` is the production specification package for Planr V1.
- `examples/real-world-flow.md`: executable real-world operator flow.
- `scripts/`: installer and release packaging scripts.
- `npm/`: the npm wrapper package.

## Module Ownership

- `src/main.rs`: process composition root. Owns top-level module wiring, process startup, database opening, error printing, and dispatch into `App`.
- `src/cli.rs`: CLI contract boundary. Owns `clap` command definitions, option parsing types, value enums, and command DTOs used by app dispatch.
- `src/app/mod.rs`: application composition boundary. Owns the `App` runtime state, top-level dispatch, shared app-local row helpers, and app submodule wiring.
- `src/app/commands.rs`: CLI use-case orchestration. Owns project, plan, map, item, link, pick, approval, log, close, review, context, search, doctor, and install command handlers.
- `src/app/flow.rs`: compound work-flow boundary. Owns evidence log writing (with heartbeat folding), the close transition core, review-request creation, the pick work packet, and the `done` command that chains them for CLI, HTTP, and MCP surfaces.
- `src/app/git_review.rs`: Git and PR review evidence boundary. Owns worktree detection, scoped changed-file provenance, PR URL context, and dirty-worktree safety projections.
- `src/app/mcp.rs`: MCP stdio boundary. Owns MCP protocol request routing, tool calls, resource reads, and prompt responses.
- `src/app/packages.rs`: package import/export boundary. Owns reusable JSON templates, preview-before-import, review artifact package import, and local-first encrypted bundle metadata.
- `src/app/http.rs`: localhost HTTP/SSE boundary. Owns HTTP request parsing, routes, SSE stream output, and HTTP response mapping.
- `src/app/repository.rs`: application data access helpers. Owns Planr query/update helpers over projects, plans, graph items, links, runs, logs, artifacts, events, approvals, search, and map projections.
- `src/app/lease.rs`: worker lease ownership. Owns the single pick query (`PickFilter`: exclude, work type, plan scope), worker ownership checks, runtime heartbeat/progress/pause state, and stale-pick detection.
- `src/app/review.rs`: review-gate application logic. Owns review annotations, feedback ingestion, evidence artifacts, review closure, and review target lookup.
- `src/app/recovery.rs`: recovery automation logic. Owns item retry policy configuration, task conditions, stale/timed-out sweeps, retry scheduling, and recovery result projections.
- `src/app/review_workspace.rs`: local review workspace boundary. Owns the browser review HTML, workspace data projection, and privacy-minimized Git diff evidence.
- `src/app/surfaces.rs`: non-CLI runtime surfaces. Owns trace, scrub, artifact, event, debug, export, and import command handlers.
- `src/app/inspection.rs`: local inspection helpers. Owns debug bundles, context/link snapshots, pick context, secret scans, export value assembly, run recording, search results, and Planr-directory import parsing.
- `src/app/audit.rs`: goal contract audit boundary. Owns the clause-by-clause `plan audit` verdict (items settled, reviews complete, approvals clear, verification logged) and its human rendering.
- `src/app/application.rs`: shared surface-mutation boundary. Owns the approval request/approve/deny, context, log, artifact, and close mutations reused verbatim by CLI, MCP, and HTTP handlers so the three surfaces cannot drift.
- `src/app/repository/`: focused data-access submodules (`item.rs`, `plan.rs`, `project.rs`, `link.rs`, `context.rs`, `evidence.rs`, `search.rs`) split out of `src/app/repository.rs` by entity ownership.
- `src/model.rs`: JSON-facing data transfer types and typed vocabulary. Owns serializable Planr DTOs plus the `ItemStatus`, `WorkType`, `LinkKind`, and `ApprovalStatus` enums with their parsing and display behavior, used by CLI JSON, MCP, HTTP, storage rows, and tests.
- `src/storage/mod.rs`: SQLite connection boundary. Owns default database path, connection setup, pragma configuration, and storage submodule exports.
- `src/storage/schema.rs`: SQLite schema boundary. Owns DDL, additive schema upgrade helpers, and schema version recording.
- `src/storage/rows.rs`: SQLite row mapping boundary. Owns row-to-DTO and row-to-JSON mapping functions.
- `src/planpack.rs`: Markdown package generation and parsing. Owns project context templates, product/build plan templates, plan metadata parsing, hashes, search body extraction, and task extraction.
- `src/integrations.rs`: agent-client integration descriptor boundary. Owns Codex, Claude Code, Cursor, MCP install metadata, MCP tool schemas, MCP resources, and MCP text response wrapping.
- `src/util.rs`: small CLI-boundary utilities. Owns ids, timestamps, path helpers, output formatting, and safe file writes.

## Boundary Rules

- Command parsing belongs in `src/cli.rs`; process startup belongs in `src/main.rs`; command execution belongs under `src/app/`.
- `src/main.rs` must stay small enough to be only a composition root. It must not own product use cases.
- `src/app/mod.rs` must stay small enough to wire runtime state and dispatch. It must not absorb app submodule behavior.
- SQLite schema belongs in `src/storage/schema.rs`; row mapping belongs in `src/storage/rows.rs`; app data access helpers belong in `src/app/repository.rs` and its submodules.
- Mutations shared by more than one surface (CLI, MCP, HTTP) belong in `src/app/application.rs`; surface handlers must call the shared helper instead of repeating SQL.
- Markdown templates belong in `planpack.rs`; command handlers should request generated file sets instead of embedding large template bodies.
- Agent install metadata and MCP schema descriptors belong in `src/integrations.rs`; client-specific strings should not drift across command handlers and docs.
- DTO and vocabulary-enum changes belong in `src/model.rs`; JSON response shapes should reuse those DTOs before adding ad hoc maps.
- Item status, work type, link kind, and approval status values are typed enums; new states must be added to the enum, not smuggled in as strings.
- Utility code must stay narrow. If a helper starts owning product behavior, move it to the owning module instead of growing `util.rs`.
- Do not add catch-all `common`, `shared`, or broad utility modules. New modules must name a durable ownership boundary.

## Single-Crate Decision

Planr remains a single crate for V1 because:

- there is one deployable binary and no separate service or reusable library boundary;
- the current behavior contract is tighter when CLI, MCP, HTTP, storage, and docs ship together;
- module-level ownership gives the needed architecture separation without duplicating Cargo settings or release packaging;
- npm, release, and external consumer tests assume one native binary named `planr`.

A Cargo workspace was tried and reverted: it produced anemic crates whose only job was being a layer, plus re-export shims in the binary. A workspace should be introduced only after a concrete deployable, reuse, compilation, or team ownership boundary exists and package/release scripts are updated in the same change.

## Future Extract Points

If Planr grows past the V1 binary shape, the first clean extraction path is:

- `planr-core`: `model.rs`, graph invariants, plan package contracts, and pure use-case types.
- `planr-storage`: `src/storage/*`, storage repositories, schema upgrades, and import/export packages.
- `planr-cli`: `src/cli.rs`, human output, and install helpers.
- `planr-server`: `src/app/http.rs`, `src/app/mcp.rs`, and runtime server adapters.

Do not extract those crates until a real reuse, compile-time, or ownership boundary exists. Every extraction must leave a real owner with runtime code, tests, and one-way dependencies.
