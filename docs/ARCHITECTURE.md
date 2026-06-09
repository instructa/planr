# Planr Architecture

Planr V1 is a single Rust binary with explicit module ownership. The crate stays small enough that a Cargo workspace would add more process overhead than value today, and there is only one deployable: the `planr` CLI. The source tree is split by ownership boundary inside that crate instead of using a premature workspace.

## Module Ownership

- `src/main.rs`: process composition root. Owns top-level module wiring, process startup, database opening, error printing, and dispatch into `App`.
- `src/cli.rs`: CLI contract boundary. Owns `clap` command definitions, option parsing types, value enums, and command DTOs used by app dispatch.
- `src/app/mod.rs`: application composition boundary. Owns the `App` runtime state, top-level dispatch, shared app-local row helpers, and app submodule wiring.
- `src/app/commands.rs`: CLI use-case orchestration. Owns project, plan, map, item, link, pick, approval, log, close, review, context, search, doctor, and install command handlers.
- `src/app/git_review.rs`: Git and PR review evidence boundary. Owns worktree detection, scoped changed-file provenance, PR URL context, and dirty-worktree safety projections.
- `src/app/mcp.rs`: MCP stdio boundary. Owns MCP protocol request routing, tool calls, resource reads, and prompt responses.
- `src/app/packages.rs`: package import/export boundary. Owns reusable JSON templates, preview-before-import, review artifact package import, and local-first encrypted bundle metadata.
- `src/app/http.rs`: localhost HTTP/SSE boundary. Owns HTTP request parsing, routes, SSE stream output, and HTTP response mapping.
- `src/app/repository.rs`: application data access helpers. Owns Planr query/update helpers over projects, plans, graph items, links, runs, logs, artifacts, events, approvals, search, and map projections.
- `src/app/review.rs`: review-gate application logic. Owns review annotations, feedback ingestion, evidence artifacts, review closure, and review target lookup.
- `src/app/recovery.rs`: recovery automation logic. Owns item retry policy configuration, task conditions, stale/timed-out sweeps, retry scheduling, and recovery result projections.
- `src/app/review_workspace.rs`: local review workspace boundary. Owns the browser review HTML, workspace data projection, and privacy-minimized Git diff evidence.
- `src/app/surfaces.rs`: non-CLI runtime surfaces. Owns trace, scrub, artifact, event, debug, export, and import command handlers.
- `src/app/inspection.rs`: local inspection helpers. Owns debug bundles, context/link snapshots, pick context, secret scans, export value assembly, run recording, search results, and Planr-directory import parsing.
- `src/model.rs`: JSON-facing data transfer types. Owns serializable Planr DTOs used by CLI JSON, MCP, HTTP, and tests.
- `src/storage/mod.rs`: SQLite connection boundary. Owns default database path, connection setup, pragma configuration, and storage submodule exports.
- `src/storage/schema.rs`: SQLite schema boundary. Owns DDL, additive schema upgrade helpers, and schema version recording.
- `src/storage/rows.rs`: SQLite row mapping boundary. Owns row-to-DTO and row-to-JSON mapping functions.
- `src/planpack.rs`: Markdown package generation. Owns project context templates and product/build plan file templates.
- `src/integrations.rs`: agent-client integration descriptors. Owns Codex, Claude Code, Cursor, and MCP install metadata.
- `src/util.rs`: small CLI-boundary utilities. Owns ids, timestamps, path helpers, output formatting, and safe file writes.

## Boundary Rules

- Command parsing belongs in `src/cli.rs`; process startup belongs in `src/main.rs`; command execution belongs under `src/app/`.
- `src/main.rs` must stay small enough to be only a composition root. It must not own product use cases.
- `src/app/mod.rs` must stay small enough to wire runtime state and dispatch. It must not absorb app submodule behavior.
- SQLite schema belongs in `src/storage/schema.rs`; row mapping belongs in `src/storage/rows.rs`; app data access helpers belong in `src/app/repository.rs`.
- Markdown templates belong in `planpack.rs`; command handlers should request generated file sets instead of embedding large template bodies.
- Agent install metadata belongs in `integrations.rs`; client-specific strings should not drift across command handlers and docs.
- DTO changes belong in `model.rs`; JSON response shapes should reuse those DTOs before adding ad hoc maps.
- Utility code must stay narrow. If a helper starts owning product behavior, move it to the owning module instead of growing `util.rs`.
- Do not add catch-all `common`, `shared`, or broad utility modules. New modules must name a durable ownership boundary.

## Single-Crate Decision

Planr remains a single crate for V1 because:

- there is one deployable binary and no separate service or reusable library boundary;
- the current behavior contract is tighter when CLI, MCP, HTTP, storage, and docs ship together;
- module-level ownership now gives the needed architecture separation without duplicating Cargo settings or release packaging;
- npm, release, and external consumer tests assume one native binary named `planr`.

A Cargo workspace should be introduced only after a concrete deployable, reuse, compilation, or team ownership boundary exists and package/release scripts are updated in the same change.

## Future Extract Points

If Planr grows past the V1 binary shape, the first clean extraction path is:

- `planr-core`: `model.rs`, graph invariants, plan package contracts, and pure use-case types.
- `planr-storage`: `src/storage/*`, storage repositories, schema upgrades, and import/export packages.
- `planr-cli`: `src/cli.rs`, human output, and install helpers.
- `planr-server`: `src/app/http.rs`, `src/app/mcp.rs`, and runtime server adapters.

Do not extract those crates until a real reuse, compile-time, or ownership boundary exists.
