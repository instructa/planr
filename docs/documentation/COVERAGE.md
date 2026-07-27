# Documentation Coverage Matrix

This is the canonical inventory that maps shipped public surfaces to repository owners and published routes. Every `/docs` target in this file must resolve to a current MDX page; `pnpm docs:verify-maintenance` enforces that rule. Retired aliases belong only in `apps/docs/redirects.mjs`, never in this current-route matrix.

## Published route inventory

The site currently owns 62 MDX routes. Next.js also emits the landing page and framework support routes during the production build.

| Section | Published routes |
| --- | --- |
| Documentation | `/docs` |
| Getting started | `/docs/getting-started`, `/docs/getting-started/why-planr`, `/docs/getting-started/installation`, `/docs/getting-started/quickstart`, `/docs/getting-started/full-lifecycle`, `/docs/getting-started/choose-your-interface` |
| For Agents | `/docs/agents`, `/docs/agents/quickstart`, `/docs/agents/prompt-recipes`, `/docs/agents/skills` |
| Integrations | `/docs/integrations`, `/docs/integrations/codex`, `/docs/integrations/claude-code`, `/docs/integrations/cursor`, `/docs/integrations/grok-build`, `/docs/integrations/generic-mcp`, `/docs/integrations/cli-only` |
| Plugins | `/docs/plugins`, `/docs/plugins/switchloom` |
| Concepts | `/docs/concepts`, `/docs/concepts/local-first-model`, `/docs/concepts/plans-and-map`, `/docs/concepts/graph-and-readiness`, `/docs/concepts/picks-and-leases`, `/docs/concepts/evidence-and-context`, `/docs/concepts/reviews-and-approvals`, `/docs/concepts/recovery-packages-and-closure` |
| Guides | `/docs/guides`, `/docs/guides/daily-worker-loop`, `/docs/guides/parallel-coordination`, `/docs/guides/handoff-and-resume`, `/docs/guides/review-and-fix-loops`, `/docs/guides/recover-interrupted-work`, `/docs/guides/packages-and-reuse`, `/docs/guides/recipes` |
| Reference | `/docs/reference`, `/docs/reference/cli`, `/docs/reference/cli-generated`, `/docs/reference/mcp`, `/docs/reference/mcp-schemas-generated`, `/docs/reference/http-api`, `/docs/reference/configuration-and-storage`, `/docs/reference/data-and-status`, `/docs/reference/outputs-and-errors`, `/docs/reference/support-matrix`, `/docs/reference/maintenance` |
| Contributing | `/docs/contributing`, `/docs/contributing/repository-setup`, `/docs/contributing/architecture`, `/docs/contributing/docs-authoring`, `/docs/contributing/testing`, `/docs/contributing/security-and-privacy` |
| Operations | `/docs/operations`, `/docs/operations/release`, `/docs/operations/versioning-and-migrations`, `/docs/operations/docs-deployment`, `/docs/operations/health-and-diagnostics`, `/docs/operations/rollback`, `/docs/operations/documentation-governance` |
| Help | `/docs/troubleshooting`, `/docs/faq` |

## Product journeys and concepts

| Surface | Canonical owner | Published route(s) |
| --- | --- | --- |
| Promise, users, non-goals | `.planr/plans/product/planr/PRODUCT_SPEC.md` | `/docs/getting-started/why-planr` |
| Install, first success, full lifecycle | manifests, installers, `src/cli.rs`, E2E tests | `/docs/getting-started/installation`, `/docs/getting-started/quickstart`, `/docs/getting-started/full-lifecycle` |
| Agent setup, public routing, and prompt recipes | typed client/prompt contracts and installed Planr skills | `/docs/agents`, `/docs/agents/quickstart`, `/docs/agents/prompt-recipes`, `/docs/agents/skills` |
| Local-first authority and boundaries | product/data specs, `src/storage/` | `/docs/concepts/local-first-model` |
| Product plans, build plans, maps | `src/planpack.rs`, `src/app/commands.rs` | `/docs/concepts/plans-and-map` |
| Items, links, statuses, readiness | `src/model.rs`, `src/app/repository/item.rs`, `src/app/lease.rs` | `/docs/concepts/graph-and-readiness` |
| Picks, leases, concurrency, progress | `src/app/lease.rs`, `src/app/flow.rs` | `/docs/concepts/picks-and-leases`, `/docs/guides/parallel-coordination` |
| Logs, contexts, artifacts, live evidence | `src/app/application.rs`, `src/app/inspection.rs` | `/docs/concepts/evidence-and-context` |
| Reviews, approvals, fix chains | `src/app/review.rs`, `src/app/flow.rs`, `src/app/application.rs` | `/docs/concepts/reviews-and-approvals`, `/docs/guides/review-and-fix-loops` |
| Recovery, conditions, packages, closure | `src/app/recovery.rs`, `src/app/packages.rs`, `src/app/flow.rs` | `/docs/concepts/recovery-packages-and-closure`, `/docs/guides/recover-interrupted-work`, `/docs/guides/packages-and-reuse` |
| Worker loop, handoff, recipes | application flow and lease owners | `/docs/guides/daily-worker-loop`, `/docs/guides/handoff-and-resume`, `/docs/guides/recipes` |

## CLI, MCP, HTTP, and data contracts

The executable and schema sources decide exact inventory. Editorial pages explain usage; generated pages carry exhaustive command and schema detail.

| Surface | Canonical owner | Published route(s) |
| --- | --- | --- |
| CLI invocation and automation rules | `src/cli.rs`, compiled help | `/docs/reference/cli` |
| Every CLI group and subcommand | compiled recursive help, reference generator | `/docs/reference/cli-generated` |
| MCP transport, resources, prompts, results | `src/app/mcp.rs`, `src/integrations.rs`, fixture | `/docs/reference/mcp` |
| Every MCP tool input schema | live MCP discovery, schema generator | `/docs/reference/mcp-schemas-generated` |
| Local HTTP/SSE and review routes | `src/app/http.rs` | `/docs/reference/http-api` |
| Environment, installers, storage, repository files | CLI/install/storage owners | `/docs/reference/configuration-and-storage` |
| DTOs, IDs, statuses, links, SQLite tables, packages | `src/model.rs`, `src/storage/schema.rs` | `/docs/reference/data-and-status` |
| JSON output, error codes, exit/recovery behavior | application and surface adapters | `/docs/reference/outputs-and-errors` |
| Platforms, clients, transports | manifests, release and integration owners | `/docs/reference/support-matrix` |
| Reference generation and synchronization | generator/check scripts and CI | `/docs/reference/maintenance` |

## Installation, clients, and safety

| Surface | Canonical owner | Published route(s) |
| --- | --- | --- |
| Homebrew, install script, npm, source build | release manifests, `scripts/install.sh`, npm wrapper | `/docs/getting-started/installation` |
| Interface selection and client differences | integration descriptors and installers | `/docs/getting-started/choose-your-interface`, `/docs/integrations` |
| Agent-led setup and autonomous handoff | typed onboarding prompts and Planr goal/loop contracts | `/docs/agents/quickstart`, `/docs/agents/prompt-recipes` |
| Optional external plugins | external lifecycle documentation and provider-neutral routing boundaries | `/docs/plugins`, `/docs/plugins/switchloom` |
| Codex | Codex manifest, generated role/install assets | `/docs/integrations/codex` |
| Claude Code | Claude manifest, generated role/install assets | `/docs/integrations/claude-code` |
| Cursor | Cursor manifest, role/skill/install assets | `/docs/integrations/cursor` |
| Grok Build | Native role/skill assets and portable project MCP config | `/docs/integrations/grok-build` |
| Generic stdio MCP | MCP server and fixture | `/docs/integrations/generic-mcp` |
| CLI-only and non-first-class hosts | prompt output and CLI | `/docs/integrations/cli-only` |
| Privacy, secret handling, localhost boundary | safety spec, `src/app/http.rs`, scrub behavior | `/docs/contributing/security-and-privacy` |
| User diagnosis and common questions | doctor/debug behavior and E2E tests | `/docs/troubleshooting`, `/docs/faq` |

## Contributor and operations coverage

| Surface | Canonical owner | Published route(s) |
| --- | --- | --- |
| Repository setup and worktree safety | manifests and `AGENTS.md` | `/docs/contributing/repository-setup` |
| Code and docs architecture ownership | `docs/ARCHITECTURE.md`, compiled source, docs contract | `/docs/contributing/architecture` |
| MDX, components, navigation, preview | `apps/docs` source and component contract | `/docs/contributing/docs-authoring` |
| Rust, docs, semantic, browser, accessibility gates | tests, scripts, CI workflows | `/docs/contributing/testing` |
| Product release and publication | `scripts/release.sh`, release workflow, `docs/RELEASE.md` | `/docs/operations/release` |
| Version synchronization and SQLite upgrades | release script, manifests, `src/storage/schema.rs` | `/docs/operations/versioning-and-migrations` |
| Alchemy static docs build and Cloudflare deployment | `apps/docs/alchemy.run.ts`, Next static export, environment contract | `/docs/operations/docs-deployment` |
| Runtime diagnosis | docs scripts, source loader, deployment config | `/docs/operations/health-and-diagnostics` |
| Docs and product rollback boundaries | immutable deployment contract, schema owner, release policy | `/docs/operations/rollback` |
| Ownership, freshness, redirects, coverage | this matrix, docs contract, IA, redirect inventory, CI | `/docs/operations/documentation-governance` |

## Audit completion checklist

- [x] All 62 current MDX routes are explicitly inventoried.
- [x] Every public product, CLI, MCP, HTTP, data, client, contributor, and operations surface has a current target and canonical source owner.
- [x] Generated CLI and MCP inventories are separated from editorial guidance and mechanically checked.
- [x] Retired aliases are isolated in `apps/docs/redirects.mjs` and resolve to a current route.
- [x] `pnpm docs:verify-maintenance` fails for missing coverage routes, missing anchors, undeclared pages, invalid redirects, dependency drift, or release-tag drift.
