# Handoffs And Stories

Planr separates live state from narrative memory.

- Live state belongs in the map.
- Completion proof belongs in logs.
- Searchable decisions belong in contexts.
- Long-form narrative belongs in story logs.

## Handoff Logs

Use `planr log add` whenever another agent, reviewer, or future session must know what happened:

```bash
planr log add --item <item-id> \
  --summary "Implemented release dry-run checks" \
  --files scripts/build-release.sh,docs/RELEASE.md \
  --cmd "scripts/build-release.sh" \
  --cmd "cargo test"
```

A useful handoff log names:

- changed files;
- commands run;
- test results;
- assumptions kept;
- decisions made;
- known remaining risk.

Do not paste private source files, secrets, tokens, or full transcripts into logs.

## Context Entries

Use context for durable discoveries:

```bash
planr context add "Release dry-runs must not edit global agent configuration." --tag constraint
planr context add "Use project-scoped MCP config for client examples." --tag decision
```

Prefer context when the fact may influence multiple future items.

## Task Notes

Use notes for local item discussion:

```bash
planr note add "Reviewer asked for an extra npm pack dry-run before closure." --item <item-id>
```

Prefer notes when the fact is not project-wide and should stay near one item.

## Story Logs

Use story logs only when map items, contexts, and logs are too thin to preserve the decision chain.

Create a story log for:

- a large architecture or ownership change;
- a multi-round review/fix sequence;
- a discovery that invalidates earlier assumptions;
- a release or migration path that future agents must understand;
- interruption-prone work where the "why" matters as much as the status.

Recommended location for repo-versioned product stories:

```text
.planr/stories/
```

Recommended location for private operator-only stories:

```text
~/.planr/<project>/stories/
```

Do not create `todo`, `in-progress`, or `finished` story folders. Status belongs to the map.

## Story Template

```markdown
# <Story Title>

Project: <project name>
Related items: <item ids>
Updated: <YYYY-MM-DD>

## Problem

## Current State

## Key Discoveries

## Decisions

## Rejected Alternatives

## Implementation Or Review History

## Verification

## Open Risks

## Next Likely Steps
```

Keep the story focused on narrative. Do not duplicate the full map.

## Recovery Order

When resuming work:

1. `git status --short`
2. `planr map show --json`
3. `planr trace item <item-id>`
4. `planr log list --item <item-id>`
5. `planr context list --item <item-id>`
6. read a relevant story log only if the trace and logs do not explain why the work exists

The story log may explain direction, but it must not override the map.
