# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

A self-contained HTML board rendered by the planr engine (`skills/planr/scripts/lib/board.mjs`) from `skills/planr/assets/board/template.html`: no framework, no network at view time, bundled fonts. The bb plugin (`bb-plugin/`) shows the same board data with bb's own components and tokens.

## Users

A product owner who runs several coding-agent threads against one product at a time. They open the board between agent runs to answer: what changed since I last looked, what waits on my approval, decision or check, what can start next, and what is holding the most work up. They often read it at a glance in a side panel (roughly 320–900 px wide), sometimes at full width.

## Product Purpose

At-a-glance status for a planning repository, plus the few human actions agents must not take: approve a goal, close a goal, mark a task checked (bb plugin only). Success: within one viewport, without scrolling, the owner sees overall state, recent changes, what waits on them and what is next, and can drill into any task's dependencies and history.

## Positioning

It is driven by the planning repo's own git history: every first-parent commit that changes a task's status is a run, so the board shows what each agent run actually changed, not just the current snapshot. Readiness, unlocks and goal progress come from the `depends` graph, computed by one engine that the CLI, the board and the plugin share.

## Operating Context

- Source of truth: a planning repository, often private and separate from the code repository. One Markdown file per task with frontmatter (`id`, `title`, `status`, `track`, `depends`, `owner`, `refs`); goals in `plans/<name>-goal.md`. No derived files are committed.
- Statuses: `open`, `in-progress`, `review`, `blocked`, `decision` (waiting on an approver), `done`, `dropped`. Tracks are project-specific.
- Agents commit task changes often, sometimes several times per hour.
- Standalone: `planr board --out <file>` or `planr board --serve` (loopback, live reload). In bb: a plugin page and a thread side panel, both refreshed live.

## Capabilities and Constraints

- Must work for any project that follows the planning-repo format; nothing project-specific is hard-coded (track order is derived or configurable).
- UI labels are English. Task titles stay in the language the planning repo uses.
- The standalone board is read-only. Human actions exist only in the bb plugin and are re-checked on the host.
- Must stay above the fold at the target width: overview and navigation visible without page scroll; long lists scroll inside their own panes.
- Sample data: `fixtures/board-data.json` (synthetic).

## Product Principles

1. Change first: what moved since the last look outranks totals.
2. Attention before inventory: blocked and decision items are never below the fold.
3. Density with order: many tasks visible at once, scannable by ID, status and track.
4. Every number drills down: counts, runs and levers filter the task view.
5. Generic over bespoke: the same page serves any planning repo of this format.

## Accessibility & Inclusion

Status is never conveyed by color alone (glyph plus label). Keyboard reachable navigation and task selection. Works in light and dark themes.
