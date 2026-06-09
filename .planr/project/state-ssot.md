# State SSOT

Default expectations for **where truth lives** in this repo. Customize for your persistence and UI model.

## Principles

- Persisted or authoritative state has one canonical owner; UI mirrors derive from it.
- Feature-local UI state stays near the feature.
- Derived state is recomputed from SSOT, not duplicated with fragile sync.

## Planning questions

Before state changes, answer:

- What is the current source of truth?
- What competing source should be removed or narrowed?
- Which layer should derive instead of store?

## Anti-patterns

- Two parallel owners for the same rule
- “Helpful” fallbacks in transport that become the real behavior
- Dead config or schema fields that no longer affect behavior

## Plan Notes

Align with your repo’s data model and API contracts when documented.
