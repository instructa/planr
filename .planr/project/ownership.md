# Ownership

Define **your** codebase’s layers so `planr-plan` / `planr-review` can place work correctly.

Replace the placeholders below with real paths (examples: `src/`, `packages/foo/`, `internal/`, `cmd/`).

## Suggested layers (customize)

| Layer | Path pattern | Owns |
|-------|----------------|------|
| Entry / UI | *(e.g. `apps/web/`, `cli/`) | User-facing surface, presentation |
| Application / API | *(e.g. `server/`, `api/`) | Use-cases, orchestration |
| Domain / core | *(e.g. `lib/`, `domain/`) | Shared rules, types, pure logic |
| Integrations | *(e.g. `adapters/`, `integrations/`) | External systems, not global product policy |

## Ownership rules

- **Runtime owner**: where the behavior runs today.
- **First-fix owner**: where the bug or duplication lives now.
- **Canonical owner**: where the logic should live after cleanup.

Do not collapse these into one vague answer.

## Wrong defaults (typical smells)

- UI owning durable server-side truth
- Thin transport layers owning domain policy
- Duplicated rules across packages without a single SSOT

## Plan Notes

Update from your repo’s architecture or CONTRIBUTING docs when available.
