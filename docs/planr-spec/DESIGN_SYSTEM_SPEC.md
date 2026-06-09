# Design System Specification

## Design Principles

- Operational, not decorative.
- Dense enough for repeated developer use.
- Log-first: status, blockers, files, and verification are always scannable.
- Calm hierarchy: avoid dashboards that obscure the next action.
- Text remains copyable and useful in terminals.

## Brand Tone

Planr should sound precise, direct, and practical:

- "picked i-api by codex-1"
- "blocked: t-schema is not done"
- "review found 2 issues"

Avoid hype, gamification, and vague success language.

## Visual Direction

- CLI/TUI: high-contrast, compact, status-color restrained.
- Web dashboard: work-focused, table/graph hybrid, no oversized hero sections.
- Cards only for repeated items, log, and item summaries.
- Radius: 6px maximum unless platform default requires otherwise.

## Color System

- Neutral background.
- Status colors:
  - ready: blue or cyan.
  - running/picked: amber.
  - done: green.
  - blocked/failed: red.
  - review: violet only as accent, not dominant palette.
- Provide no-color fallback.

## Typography

- CLI: terminal default.
- Dashboard: system UI font for text, monospace for ids/commands.
- No viewport-scaled text.
- Long ids and paths must wrap or truncate with copy affordance.

## Spacing And Layout

- Item rows must keep stable height in tables.
- Graph nodes must have predictable min/max widths.
- Side panels should show details without covering the primary queue.
- Mobile dashboard, if implemented, uses stacked list/detail, not dense graph canvas.

## Component System

- Status badge.
- Item row.
- Dependency edge.
- Plan link.
- Log card.
- Command log block.
- Review finding row.
- Agent run row.
- Search result row.
- Doctor diagnostic item.

## Motion And Haptics

- No required motion.
- Dashboard transitions must respect reduced motion.
- Live updates may pulse once but must not animate continuously.

## Iconography And Illustration

- Use simple icons only for status, actions, and diagnostics.
- Do not use decorative illustrations in the product UI.

## Data Visualization Rules

- Graph view must distinguish containment from dependency edges.
- Critical path must be visually separable from general edges.
- Hidden nodes must be indicated with counts.
- Graph view must have an equivalent text/table representation.

## Accessibility Requirements

- REQ-DES-001: All dashboard actions must be keyboard reachable.
- REQ-DES-002: Color cannot be the only status indicator.
- REQ-DES-003: Command blocks must be selectable and copyable.
- REQ-DES-004: Graph state must be available as text for screen readers.

## Platform-Specific UI Conventions

- CLI follows Unix command conventions.
- MCP prompt names are stable and descriptive.
- Cursor/Claude/Codex instructions should use each client's standard config paths and avoid hidden global edits unless requested.

## Do Not Do

- Do not build a marketing landing page as the product surface.
- Do not use a single purple gradient theme.
- Do not hide blockers behind cheerful progress summaries.
- Do not put UI cards inside cards.
