# UX Flows

## Navigation Model

Planr V1 is CLI-first with optional TUI/dashboard. Public commands use Planr-native vocabulary:

- `planr project`: setup and integration.
- `planr plan`: reviewable Markdown plans for product specs, build contracts, migrations, and review packages.
- `planr map`: live graph inspection and mutation.
- `planr item`: item creation and detail.
- `planr pick`: atomically pick the next ready item.
- `planr log`: record files, commands, tests, and handoff proof.
- `planr review`: review/fix/approval loop.
- `planr close`: close an item with log.
- `planr doctor`: diagnostics.

## Screen Map

### CLI

- Project status.
- Item detail.
- Ready queue.
- Critical lane.
- Plan list/detail.
- Map summary.
- Log detail.
- Doctor report.

### Optional TUI/Dashboard

- Project overview.
- Graph view.
- Ready/running/blocked map lanes.
- Plan viewer.
- Item detail.
- Log/review panel.
- Agent runs panel.
- Diagnostics panel.

## State Machine

```text
NO_PROJECT -> INITIALIZED
Trigger: planr project init
Guard: repo path writable
Side effects: create database and .planr pack
Failure behavior: report blocked path and no partial overwrite

INITIALIZED -> PRODUCT_PLANNED
Trigger: planr plan new
Guard: project exists
Side effects: create product plan package
Failure behavior: show validation errors

PRODUCT_PLANNED -> BUILD_PLANNED
Trigger: planr plan split
Guard: product plan has selectable slice
Side effects: create focused build plan
Failure behavior: show missing decisions

BUILD_PLANNED -> MAPPED
Trigger: planr map build --from <plan>
Guard: plan acceptance criteria are parseable
Side effects: create linked map items
Failure behavior: show unmapped requirements

MAPPED -> ITEM_PICKED
Trigger: planr pick
Guard: at least one ready item
Side effects: atomic pick, start run context
Failure behavior: show blockers or no-ready status

ITEM_PICKED -> EVIDENCE_RECORDED
Trigger: planr log add
Guard: log fields valid
Side effects: write log and attach to item/run
Failure behavior: keep item running/blocked with diagnostic

EVIDENCE_RECORDED -> ITEM_CLOSED
Trigger: planr close
Guard: required log and reviews are satisfied
Side effects: close item, promote downstream items
Failure behavior: keep item running/blocked with diagnostic

ITEM_CLOSED -> REVIEW_READY
Trigger: review policy requires review
Guard: code item completed
Side effects: create or unlock review item
Failure behavior: parent stays incomplete
```

## Onboarding Flow

1. User runs `planr project init`.
2. Planr detects repo, Git state, old `.planr` artifacts, and available agent clients.
3. Planr creates project pack and database.
4. Planr offers integration commands:
   - Codex MCP/config.
   - Claude Code MCP config.
   - Cursor `.cursor/mcp.json`.
5. User runs `planr doctor --client all`.

Acceptance criteria:

- REQ-UX-001: Init output must clearly distinguish created files, detected files, and skipped files.
- REQ-UX-002: Doctor output must show each agent integration as pass, fail, warning, or not installed.

## Product Plan Flow

1. User runs `planr plan new "App idea"`.
2. Planr captures product intent, platform, users, data sensitivity, AI, monetization, integrations, and launch constraints.
3. Planr creates a Markdown plan package: product spec, UX, architecture, ADRs, safety/privacy/security, API/data model, implementation specs, QA, release, and task checklist.
4. Planr marks assumptions and open decisions explicitly.
5. User runs `planr plan check`.

Acceptance criteria:

- REQ-UX-010: A created plan must include product, UX, architecture, safety/privacy/security, data/API, QA, release, and task files unless explicitly scoped down.
- REQ-UX-011: Plan checks must report missing required sections, open decisions, and validation warnings.

## Implementation Plan Flow

1. User or agent runs `planr plan split <plan> --slice "MVP backend"`.
2. Planr creates a focused Markdown plan with source, scope decision, ownership target, phases, verification, and acceptance criteria.
3. Planr links the build plan to source product plan requirement IDs.
4. User runs `planr plan check`.

Acceptance criteria:

- REQ-UX-012: A build plan must be narrow enough to seed executable map items.
- REQ-UX-013: Plan linking must show source plan id, requirement ids, derived plan path, and relationship.

## Map Seeding Flow

1. User runs `planr map build --from <plan>`.
2. Planr creates candidate map items and links.
3. User accepts, edits, or rejects candidates.
4. Accepted items enter the map as pending/ready.

Acceptance criteria:

- REQ-UX-014: Plan work lists must not silently become live map work without acceptance.
- REQ-UX-015: Seeded items must retain links to their source product or build plan requirements.

## Agent Execution Flow

1. Agent calls `planr_pick_item` or `planr pick`.
2. Planr returns item, linked plan context, upstream handoff, relevant contexts, and conflicts.
3. Agent works in repo.
4. Agent records log.
5. Agent closes the item when log and reviews allow it.
6. Planr promotes downstream items.
7. If review is required, a review item becomes ready.

Acceptance criteria:

- REQ-UX-020: The pick response must fit in an LLM context window by default and include links for deeper reads.
- REQ-UX-021: Closure must show unlocked items and remaining blockers.
- REQ-UX-022: Agents must treat parent gate items as completion gates and work executable child items when a child implementation or test item exists.

## Parent Gate Flow

1. User or agent creates a parent item for the material change.
2. User or agent breaks the parent into implementation or test child work with `planr item breakdown`.
3. User or agent requests review on the implementation or test child after evidence exists.
4. Downstream top-level work depends on the parent gate when review cleanliness matters.
5. Parent closes only after child work closes and review findings have either passed or been converted into fix/follow-up review work.

Acceptance criteria:

- REQ-UX-025: Parent item detail must make clear whether open children or reviews block closure.
- REQ-UX-026: Recovery views must show the child implementation, review, logs, and blockers needed to continue safely.

## Review/Fix Flow

1. Review item is picked.
2. Reviewer inspects linked plan, log, and scoped Git diff.
3. Reviewer returns verdict:
   - complete;
   - not complete with findings;
   - unclear/partially verified.
4. Findings create fix item and follow-up review item.
5. Parent closes only when review chain passes.

Acceptance criteria:

- REQ-UX-030: Review findings must be visible in item detail and log detail.
- REQ-UX-031: Parent completion must explain which review item blocks it.

## Handoff And Story Flow

1. Agent records proof with `planr log add`.
2. Agent records reusable discoveries with `planr context add`.
3. Agent records task-local coordination with `planr note add`.
4. If the decision chain is too long for logs and contexts, the operator creates or updates a story log.

Acceptance criteria:

- REQ-UX-035: Logs must remain the proof surface for closure.
- REQ-UX-036: Context entries must be searchable and safe to expose to future agents.
- REQ-UX-037: Story logs must be documented as narrative memory and must not override map state.

## Error, Empty, Offline, Loading States

- No project: show `planr project init`.
- No ready item: show blockers and critical lane.
- Database locked: show retry and owning process if known.
- Plan parse error: show file path, line if available, and keep graph usable.
- MCP client missing: show manual config.
- Agent command failed: record failed run and keep task recoverable.

## Settings, Export, Delete, Account Flows

- No account settings in V1.
- Settings live in `.planr/config.toml` or user config.
- Export includes database dump, plan files, reviews, and references.
- Delete requires explicit project id or path and confirmation.

## Accessibility Behavior

- CLI output must support `--json` and no-color mode.
- TUI/dashboard must support keyboard navigation, visible focus, screen-reader labels, and reduced motion.

## Localization Behavior

- V1 command output is English.
- User-authored plan content may be any language.
- Error codes are stable and machine-readable.

## Acceptance Criteria

- REQ-UX-900: Every primary flow must be executable without a web UI.
- REQ-UX-901: Every mutation must have a JSON mode for automation.
- REQ-UX-902: Every blocked state must show the next actionable command or reason.
