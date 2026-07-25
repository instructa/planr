# Planr Documentation Contract

Status: approved implementation contract for the Planr documentation site
Last audited: 2026-07-17
Scope owner: `apps/docs` (site and English product documentation)

This contract turns the current product, runtime, and repository documentation into one maintainable public documentation system. It is intentionally separate from the product specification package: `.planr/plans/product/planr/` remains the product source of truth, while the site explains the released product to users and contributors.

## Source hierarchy

When sources disagree, authors must use this order and disclose the disagreement instead of silently choosing convenient copy:

1. **Product intent and invariants:** `AGENTS.md` and `.planr/plans/product/planr/`, especially `PRODUCT_SPEC.md`, `TECH_ARCHITECTURE.md`, and `API_AND_DATA_MODEL.md`.
2. **Released executable behavior:** compiled `planr --help` output, `src/cli.rs`, `src/app/mcp.rs`, `src/app/http.rs`, `src/model.rs`, and the tested fixture `docs/fixtures/mcp-contract.json`. Runtime sources decide whether a command, option, schema, endpoint, state, or error exists in the current release.
3. **Distribution contract:** `Cargo.toml`, `package.json`, `pnpm-workspace.yaml`, release workflows, installers, and release tests. These decide supported versions, artifacts, operating systems, and install commands.
4. **Existing explanatory material:** `README.md`, `plugins/planr/skills/`, and examples. These explain and point at the site; they are not independent sources of truth.
5. **External references:** official upstream documentation and inspected local projects may influence structure and implementation, but never define Planr behavior.

If product intent is ahead of runtime, the public page must label the behavior as planned or omit it from executable instructions. If runtime is ahead of an older specification sentence, the public page documents the tested runtime and the discrepancy is added to the conflict register below.

## Canonical terminology

Use these terms exactly. Prefer the lowercase form in prose unless it starts a sentence.

| Term | Public definition | Do not substitute |
| --- | --- | --- |
| Planr | The local-first planning and execution coordination product. | project manager, cloud orchestrator |
| project | One repository or multi-root workspace tracked by Planr. | workspace when referring to the stored object |
| product plan | A Markdown specification package that captures product intent. | PRD when the broader package is meant |
| build plan | A focused Markdown implementation contract for a buildable slice. | task list |
| map | The authoritative live dependency graph. | board, checklist |
| item | A node in the map with status, work type, ownership, and evidence. | ticket, task when referring to the stored object |
| link | A directed relation between items, normally `blocks`. | dependency edge in introductory prose |
| pick | An atomic lease of one ready item to one worker. | assignment, claim unless explaining the concept |
| worker | The identity holding the active pick. | agent when ownership specifically matters |
| log | Durable evidence or progress attached to an item. | transcript |
| verification log | A log with kind `verification` that records a live oracle. | test result alone |
| review | A graph gate that checks evidence and can create fix/follow-up work. | approval; approvals are a separate human gate |
| approval | An explicit requested/approved/denied human decision on an item. | review |
| context | A durable discovery, decision, constraint, or goal contract. | memory |
| artifact | Item-linked or project evidence such as a report, screenshot, or review file. | attachment when referring to the stored object |
| recovery sweep | A preview/apply operation for stale, timed-out, and retryable work. | automatic retry loop |
| package | A reusable export/import bundle of graph and optional Planr artifacts. | backup unless that is the user's intent |
| routing declaration | Provider-neutral repository data in `.planr/agents.toml` or `.planr/policy.toml`. | model config |
| goal contract | Durable Planr context defining the completion oracle and stop condition. | prompt-only instruction |

The canonical lifecycle is always:

```text
idea -> product plan -> build plan -> map -> pick -> log -> review/evidence -> recovery/package -> close
```

## Selected documentation stack

The docs worker must create `apps/docs` in the existing pnpm workspace and pin direct dependencies exactly. The lockfile remains the transitive dependency authority.

| Component | Selected version | Rationale and evidence |
| --- | --- | --- |
| Fumadocs Core | `16.11.5` | npm `latest` on 2026-07-17; supplies source loading and built-in Orama search. Its peers accept Next.js `16.x` and React `^19.2.0`. |
| Fumadocs UI | `16.11.5` | npm `latest` on 2026-07-17; supplies accessible layouts and docs components. Its peer contract requires Core exactly `16.11.5`, Next.js `16.x`, and React `^19.2.0`. |
| Fumadocs MDX | `15.2.0` | npm `latest` on 2026-07-17; its major version is independent from Core/UI. Its peers accept Core `^16.7.0`, Next.js `^15.3.0 || ^16.0.0`, and React `^19.2.0`. |
| Next.js | `16.2.10` | Current stable Next.js 16 line required by the official Fumadocs Next.js guide. |
| React and React DOM | `19.2.7` | npm `latest` on 2026-07-17; satisfies the selected Fumadocs packages' `^19.2.0` peer requirement and Next.js 16's React 19 range. |
| Tailwind CSS | `4.x`, exact patch chosen at scaffold time | Fumadocs UI supports Tailwind 4 only; record the resolved exact patch in `apps/docs/package.json`. |
| Node.js | `>=22` for the docs workspace | Official Fumadocs quickstart requires Node 22. This does not change the published Planr CLI npm wrapper's Node 18 runtime contract. |
| Package manager | repository-pinned pnpm `11.5.3` | Avoids a second package manager and keeps workspace scripts reproducible. |

Evidence captured from official upstream sources:

- Fumadocs quickstart and Node requirement: <https://www.fumadocs.dev/docs>
- Fumadocs manual Next.js setup: <https://www.fumadocs.dev/docs/manual-installation/next>
- Fumadocs UI themes and Tailwind 4 contract: <https://www.fumadocs.dev/docs/ui/theme>
- Fumadocs layout API: <https://www.fumadocs.dev/docs/ui/layouts/docs>
- Fumadocs deployment constraints: <https://www.fumadocs.dev/docs/deploying>
- Alchemy v2 Cloudflare setup: <https://alchemy.run/cloudflare/setup/>
- Alchemy v2 frontend resources: <https://alchemy.run/cloudflare/frontend/frontends/>
- Alchemy v2 domains and DNS: <https://alchemy.run/cloudflare/networking/domains/>
- Stable package versions and peer ranges were replayed from the npm registry with `npm view <package>@latest version peerDependencies --json` on the audit date. The selected Core/UI/MDX/Next.js set satisfies every declared peer range.

Do not use `latest`, caret, tilde, or wildcard ranges for direct docs dependencies. A later upgrade is an explicit, reviewed dependency change with build, search, and browser verification.

## Architecture decisions

### DOC-ADR-001: One first-class workspace app

Create one English documentation application at `apps/docs`. It owns the public landing page, guides, reference, search, metadata, and deployment. The migration completed on 2026-07-25: the flat topic guides under `docs/` were removed once the site owned their topics, so no topic has a second maintained copy. `docs/` retains only non-site material — architecture ownership, the release runbook, frozen contracts, documentation governance, and test fixtures.

### DOC-ADR-002: Next.js App Router with local MDX

Use Next.js 16 App Router, Fumadocs MDX, and content under `apps/docs/content/docs`. This is the most direct officially documented Fumadocs path and gives Planr a conservative deployment surface. Use `source.config.ts`, a generated `.source` directory, `fumadocs-core/source`, the Fumadocs root provider, and the standard docs catch-all route.

Rejected for this scope:

- TanStack Start: AgentRig proves it is viable, but it adds routing/build decisions that Planr does not otherwise own and is not needed for a documentation-only app.
- A bespoke Rust-rendered site or an external routing-catalog static implementation: neither supplies the requested polished docs authoring/search system.

### DOC-ADR-003: Deploy direct static assets with Alchemy on Cloudflare

Use an Alchemy v2 Effect stack as the committed deployment owner. Next.js exports every human and agent-readable route, Fumadocs search runs from a build-time Orama database in the browser, and `Cloudflare.Website.StaticSite` serves normal pages and files directly. A tiny edge worker is limited to exact legacy aliases and machine-readable Markdown, search, and LLM paths; it redirects, fixes content types, or delegates immediately to the asset binding. Bind `planr.so` only when the Alchemy stage is `prod`; development and preview stages use generated URLs. The Cloudflare zone must already exist in the authenticated account. Local credentials live in the Alchemy profile, shared state uses `Cloudflare.state()`, and CI verifies the complete static artifact and Wrangler configuration without deploying it. This avoids making public documentation availability depend on a paid application Worker CPU allowance.

### DOC-ADR-004: Use Fumadocs primitives before copying components

Adopt `DocsLayout`, page tree navigation, breadcrumbs, table of contents, built-in search, MDX components, theme support, and metadata APIs. Customize with documented props, slots, stable ids/data attributes, and Planr design tokens. Do not fork layout components on day one; Fumadocs explicitly warns that copied components stop receiving upstream UI updates.

### DOC-ADR-005: Navigation is explicit and task-first

Use directory `meta.json` files to make the navigation order deliberate. The first paths are evaluation, installation, quickstart, and a complete lifecycle tutorial. Concepts explain the mental model; guides solve tasks; reference mirrors executable surfaces; contributor and operations pages explain maintenance.

### DOC-ADR-006: Reference is generated or mechanically checked

The CLI and MCP inventories must be derived from compiled help and the tested MCP fixture/schema. Hand-authored explanations can enrich them, but CI must fail when a public command/tool/resource/prompt lacks a target page. HTTP routes must be extracted or checked against `src/app/http.rs`. JSON examples must be replayed by tests or fixtures.

### DOC-ADR-007: AgentRig is structural inspiration, not a template dependency

The requested path `~/projects/agentrig-mono/agentrig/apps/docs` no longer exists. The active inspected reference is `~/projects/agentrig-mono/agentrig-public/apps/docs`.

Adopted patterns:

- an `apps/docs` workspace boundary;
- local MDX collections plus `meta.json` page trees;
- an overview with audience-specific cards and quick paths;
- compact installation, integration, guide, contribution, and reference sections;
- a shared layout options module, source loader, MDX component registry, search endpoint, explicit 404/error UI, and theme control.

Rejected patterns:

- direct dependencies set to `latest`;
- product-specific TanStack/Alchemy infrastructure;
- manually curated CLI pages without a drift check;
- copying AgentRig wording, branding, images, or product taxonomy.

## Conflict and gap register

These findings are explicit inputs to implementation and content review.

| ID | Finding | Resolution in the site |
| --- | --- | --- |
| GAP-001 | Resolved on 2026-07-25: the flat `docs/CLI_REFERENCE.md` was removed after `/docs/reference/cli` plus the mechanically checked `/docs/reference/cli-generated` took over command coverage. | Keep command inventory generated from the binary; never reintroduce a hand-maintained command list. |
| GAP-002 | `.planr/plans/product/planr/PRODUCT_SPEC.md` still calls Rust and the HTTP server open decisions, but the repository ships a Rust binary and `planr serve`. | Document current released behavior; update product specs only in a separately scoped product-spec change. |
| GAP-003 | Resolved on 2026-07-18: `docs/ARCHITECTURE.md` now describes Planr as one Rust binary plus wrappers/docs, with provider-neutral routing declarations owned in Core and external routing lifecycle outside Planr. | Keep architecture ownership aligned with current runtime modules and do not reintroduce routing-package ownership wording. |
| GAP-004 | Resolved on 2026-07-25: the flat install guide was removed, so `/docs/getting-started/installation` is the single install owner. | Say “Homebrew recommended on macOS”; GitHub Releases are the canonical artifact source; npm is the cross-package-manager native-binary path. |
| GAP-005 | The CLI npm wrapper supports Node 18, while latest Fumadocs requires Node 22. | Keep separate requirement callouts: Planr CLI Node 18; docs contributors Node 22. |
| GAP-006 | Product personas mention Gemini CLI and generic clients, but install helpers exist only for Codex, Claude Code, and Cursor. | Give the three supported clients first-class setup pages; route Gemini/opencode/other tools through clearly labeled generic CLI or stdio MCP instructions. Do not imply a native installer. |
| GAP-007 | `planr install` uses the subcommand `claude`, while product prose often says “Claude Code”. | Use “Claude Code” in headings and `planr install claude` in commands. |
| GAP-008 | Product specs mention optional HTTP/SSE and “streamable HTTP” in client expectations, but the implemented MCP transport is stdio; `planr serve` is a separate localhost HTTP/SSE API and review UI. | Keep MCP transport and local HTTP API separate. Do not advertise streamable-HTTP MCP support unless implemented and tested. |
| GAP-009 | Existing flat Markdown pages overlap heavily across README, install, skills, operating model, and client guides. | Assign one canonical site page per concept/task; other pages link to it instead of restating the procedure. |
| GAP-010 | The initial repository had no website package, search index, redirect policy, link checker, freshness owner, or browser accessibility gate. | Implemented in `apps/docs`: Fumadocs search, executable redirects, maintenance drift checks, governance ownership, and production-browser axe evidence. |
| GAP-011 | The requested AgentRig path is stale. | Use `agentrig-public/apps/docs` only as recorded prior art; never make it a build dependency. |
| GAP-012 | Windows native assets are not in the public install contract. | Installation clearly labels macOS/Linux native support and WSL/source alternatives; no unsupported Windows-native promise. |
| GAP-013 | `opencode` appears in README but is not a first-class install target. | Include it only as an example on the generic CLI/MCP page, with no plugin or installer guarantee. |
| GAP-014 | Resolved on 2026-07-18: Planr no longer documents a routing package CLI, catalog, compiler, bundle, or application lifecycle as current product surface. | Document only provider-neutral declarations, policy checks, pick-packet routing, and route evidence; external tools such as Switchloom remain outside Planr execution. |

## Explicit exclusions

- No localization in the initial release; English is canonical. The content structure must remain localization-ready.
- No hosted account, analytics, feedback database, or cloud search service.
- No OpenAPI rendering until Planr exposes a maintained OpenAPI document.
- No documentation for unimplemented cloud sync, team dashboard, native Windows artifact, Gemini installer, or streamable-HTTP MCP transport.
- No automatic execution of destructive examples. Preview-first examples are mandatory for delete, cancellation, replan, recovery apply, scrub, and import.
- No copying of AgentRig assets, copy, or branding.

## Definition of maintained documentation

A public surface is documented only when it has a target route in `COVERAGE.md`, a canonical owner, a tested example or schema source where applicable, and a link from the page tree. A page is release-ready only when it has a title, description, audience/purpose, prerequisites, safe runnable examples, expected outcomes, failure/recovery guidance, and next steps.
