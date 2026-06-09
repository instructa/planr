# Constraints

This file captures non-negotiable planning and implementation constraints for planr workflows.

## Hard-Cut Product Policy

Default stance:

- keep one canonical current-state implementation
- delete or narrow fallback, compatibility, shim, bridge, adapter, and dual-path behavior when the task calls for a hard cut
- when only one meaningful current-state behavior remains, delete the surrounding concept end-to-end instead of preserving a one-value enum, one-option selector, dead config key, or pass-through field
- prefer fail-fast diagnostics and explicit recovery steps
- use invalid-state diagnostics only on boundaries that still legitimately exist; do not keep deleted concepts around solely to reject their former values

Do not introduce migration glue or second current-state paths unless the user explicitly asks for transition support.

## Scope Discipline

- keep owned scope explicit
- do not silently widen into adjacent cleanup
- do not treat unrelated dirty files as part of the task
- if mixed authorship makes scope ambiguous, stop and clarify

## Owner Discipline

- thin shells stay thin
- reusable policy should not get stranded in transport or glue layers
- integration modules own external wiring, not global domain policy
- UI-only state should not become the SSOT for server or shared domain truth

## Fresh-Repo Bootstrap Rule

If a repo does not already have equivalent durable context for:

- product direction
- ownership boundaries
- critical flows
- state ownership
- quality gates

then `planr-plan` must create or request the `.planr/project/` pack before making strong architecture or ownership decisions.

## Documentation Bias

Prefer:

- durable repo-local docs
- explicit contracts
- explicit verification notes

over:

- ephemeral chat context
- implied assumptions
- checklist-only claims

## Plan Notes

Align with repo policy docs (e.g. `AGENTS.md`, contributing guides, or a local “hard cut / no dual paths” rule) when present.
