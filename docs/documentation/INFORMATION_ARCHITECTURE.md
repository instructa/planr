# Documentation Information Architecture

This file records the implemented route and navigation contract. Routes are public interfaces: rename one only after adding a tested legacy alias to `apps/docs/redirects.mjs`.

## Audiences and required journeys

| Audience | Required path | Success signal |
| --- | --- | --- |
| evaluator | `/` -> `/docs` -> `/docs/getting-started/why-planr` | Understands the local-first map and evidence model. |
| first-time user | `/docs/getting-started/installation` -> `/docs/getting-started/quickstart` | Installs, initializes, inspects, and diagnoses Planr. |
| agent-led user | `/docs/agents/quickstart` -> `/docs/agents/prompt-recipes` | Completes safe setup, starts with `$planr`, and keeps autonomous execution plan-bound. |
| lifecycle user | `/docs/getting-started/full-lifecycle` | Completes plan, map, work, evidence, and review. |
| worker/operator | `/docs/guides/daily-worker-loop` -> `/docs/guides/parallel-coordination` | Picks without collisions and records durable evidence. |
| reviewer | `/docs/concepts/reviews-and-approvals` -> `/docs/guides/review-and-fix-loops` | Closes a complete gate or creates a bounded fix chain. |
| client integrator | `/docs/integrations` -> one client page | Connects through a supported CLI or stdio MCP path. |
| plugin operator | `/docs/plugins` -> one plugin page | Uses an optional external lifecycle without confusing it with Planr Core ownership. |
| API toolsmith | `/docs/reference/mcp` or `/docs/reference/http-api` | Uses the current schema and transport boundary. |
| contributor | `/docs/contributing` -> setup/architecture/testing | Places and verifies a change without tribal knowledge. |
| maintainer | `/docs/operations` | Can release, deploy, diagnose, roll back, and review freshness. |

## Implemented route tree

The 62 MDX files below are the current page tree and must agree with every `meta.json` file and `COVERAGE.md`.

```text
/docs
├── getting-started
│   ├── why-planr
│   ├── installation
│   ├── quickstart
│   ├── full-lifecycle
│   └── choose-your-interface
├── agents (For Agents)
│   ├── quickstart
│   ├── prompt-recipes
│   └── skills
├── integrations
│   ├── codex
│   ├── claude-code
│   ├── cursor
│   ├── grok-build
│   ├── generic-mcp
│   └── cli-only
├── plugins
│   └── switchloom
├── concepts
│   ├── local-first-model
│   ├── plans-and-map
│   ├── graph-and-readiness
│   ├── picks-and-leases
│   ├── evidence-and-context
│   ├── reviews-and-approvals
│   └── recovery-packages-and-closure
├── guides
│   ├── daily-worker-loop
│   ├── parallel-coordination
│   ├── handoff-and-resume
│   ├── review-and-fix-loops
│   ├── recover-interrupted-work
│   ├── packages-and-reuse
│   └── recipes
├── reference
│   ├── cli
│   ├── cli-generated
│   ├── mcp
│   ├── mcp-schemas-generated
│   ├── http-api
│   ├── configuration-and-storage
│   ├── data-and-status
│   ├── outputs-and-errors
│   ├── support-matrix
│   └── maintenance
├── contributing
│   ├── repository-setup
│   ├── architecture
│   ├── docs-authoring
│   ├── testing
│   └── security-and-privacy
├── operations
│   ├── release
│   ├── versioning-and-migrations
│   ├── docs-deployment
│   ├── health-and-diagnostics
│   ├── rollback
│   └── documentation-governance
├── troubleshooting
└── faq
```

Each named section also has its own index route. The application additionally owns `/`, `/api/search`, the custom not-found page, metadata routes, and static assets; those are application routes rather than MDX navigation nodes.

## Navigation and page contracts

- Root and section `meta.json` files explicitly order every page.
- Guides state prerequisites, outcome, failure recovery, and next action.
- Reference pages name their generated or compiled source.
- The For Agents section owns the shared setup and prompt journey; integration pages own client-specific technical detail and render the same typed recipe rather than copying it.
- Getting Started remains the manual, CLI-first journey.
- Search indexes the same Fumadocs source tree used by navigation.
- English is canonical for this release.

## Redirect policy

`apps/docs/redirects.mjs` is the one executable inventory of retired public site aliases. `apps/docs/worker.mjs` consumes that inventory and returns permanent redirects while preserving query strings; Alchemy derives the exact worker-first paths from the same inventory. The maintenance verifier enforces unique sources, current destinations, absence of alias/source collisions, and documentation of both sides here.

The inventory covers these retired route families:

| Retired alias | Current destination |
| --- | --- |
| `/docs/concepts/mental-model` | `/docs/concepts/local-first-model` |
| `/docs/concepts/plans` | `/docs/concepts/plans-and-map` |
| `/docs/concepts/map-items-and-links` | `/docs/concepts/graph-and-readiness` |
| `/docs/concepts/statuses-and-readiness` | `/docs/concepts/graph-and-readiness` |
| `/docs/concepts/logs-and-artifacts` | `/docs/concepts/evidence-and-context` |
| `/docs/concepts/evidence-and-reviews` | `/docs/concepts/reviews-and-approvals` |
| `/docs/concepts/approvals` | `/docs/concepts/reviews-and-approvals` |
| `/docs/concepts/context-and-recall` | `/docs/concepts/evidence-and-context` |
| `/docs/concepts/recovery-and-packages` | `/docs/concepts/recovery-packages-and-closure` |
| `/docs/concepts/routing-and-policy` | `/docs/reference/configuration-and-storage` |
| `/docs/guides/new-product` | `/docs/getting-started/full-lifecycle` |
| `/docs/guides/existing-project-work` | `/docs/guides/recipes` |
| `/docs/guides/autonomous-goals` | `/docs/getting-started/full-lifecycle` |
| `/docs/guides/multi-agent-coordination` | `/docs/guides/parallel-coordination` |
| `/docs/guides/review-and-fix-loop` | `/docs/guides/review-and-fix-loops` |
| `/docs/guides/interruptions-and-recovery` | `/docs/guides/recover-interrupted-work` |
| `/docs/guides/import-export-and-templates` | `/docs/guides/packages-and-reuse` |
| `/docs/guides/local-review-workspace` | `/docs/guides/recipes` |
| `/docs/guides/host-hooks` | `/docs/integrations` |
| `/docs/guides/model-routing` | `/docs/reference/configuration-and-storage` |
| `/docs/reference/cli/index` | `/docs/reference/cli` |
| `/docs/reference/cli/project-and-plans` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/map-items-and-links` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/picks-logs-and-close` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/reviews-and-approvals` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/context-search-and-recovery` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/agents-routing-and-policy` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/install-prompt-and-server` | `/docs/reference/cli-generated` |
| `/docs/reference/cli/artifacts-events-and-packages` | `/docs/reference/cli-generated` |
| `/docs/reference/mcp/tools` | `/docs/reference/mcp-schemas-generated` |
| `/docs/reference/mcp/resources` | `/docs/reference/mcp` |
| `/docs/reference/mcp/prompts` | `/docs/reference/mcp` |
| `/docs/reference/configuration-and-environment` | `/docs/reference/configuration-and-storage` |
| `/docs/reference/storage-and-generated-files` | `/docs/reference/configuration-and-storage` |
| `/docs/reference/data-model-and-statuses` | `/docs/reference/data-and-status` |
| `/docs/reference/json-and-errors` | `/docs/reference/outputs-and-errors` |
| `/docs/reference/routing-bundles` | `/docs/reference/configuration-and-storage` |
| `/docs/reference/platform-support` | `/docs/reference/support-matrix` |

Repository Markdown files are not former website URLs, so a Next.js redirect can never intercept a repository path. The flat topic guides that once duplicated site pages were removed on 2026-07-25; `README.md` and the remaining repository files link to the current site instead of restating it. `docs/` now keeps only what the site does not own: `docs/ARCHITECTURE.md`, `docs/RELEASE.md`, `docs/contracts/`, `docs/documentation/`, and `docs/fixtures/`.

To retire another route: add exactly one alias, choose the closest current outcome page, update this table, repair internal links, run the maintenance and browser gates, and retain the redirect while supported inbound links may exist. Never redirect an unknown path to a generic landing page merely to hide a 404.
