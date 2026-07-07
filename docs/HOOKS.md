# Host Hooks

`planr install codex|claude|cursor` wires Planr into the host's native hook system **by default** — skip it with `--no-hooks`. The hooks solve one problem: loop state must survive the moments context is created or destroyed. Every new session and every context compaction gets the map state injected automatically via `planr prime`, so recovery stops being discipline and becomes mechanism.

## What gets installed

| Host | File | Events | Runs |
| --- | --- | --- | --- |
| Cursor | `.cursor/hooks.json` | `sessionStart` | `planr prime --cursor-json` |
| | | `subagentStop` | `.cursor/hooks/planr-evidence-guard.sh` |
| Claude Code | `.claude/settings.json` | `SessionStart` (matcher `startup\|resume\|compact`) | `planr prime --hook-json` |
| Codex CLI | `.codex/hooks.json` | `SessionStart` | `planr prime` |

`planr prime` prints one compact, bounded state block (project, map counts, your held items with log status, the goal contract, registry presence, and the next command) — deterministic and read-only. `--hook-json` wraps it in the Claude Code SessionStart envelope (`hookSpecificOutput.additionalContext`); `--cursor-json` emits Cursor's `additional_context` shape.

Only session-start events are wired, deliberately: they are the one place all three hosts inject hook output as context, and Claude's `compact` source plus post-compaction session starts cover the compaction case. Pre/post-compaction events (`preCompact`, `PreCompact`, `PostCompact`) cannot restore model context and are not used.

The evidence guard is advisory and identity-scoped: a Cursor subagent that stops while *its own* pick (matched via `PLANR_WORKER_ID`/`PLANR_SESSION_ID`) has no completion log gets one follow-up message naming the item and the two ways out (`planr done ...` or `planr pick release ...`). Without an explicit worker identity it stays silent rather than steering the wrong agent toward foreign items. It never blocks.

## Design rules

- **Fail open, always.** Every hook command ends in `|| true` with a 10-second timeout; a missing, broken, or uninitialized planr never blocks a session. In a repo without a Planr database, `planr prime` exits silently and creates nothing.
- **Additive merge, planr owns only its own entries.** Existing hook files keep every foreign entry; planr entries are reconciled on re-install (current ones untouched, outdated ones upgraded in place, entries under retired events removed) — so `planr install <client>` after an update also refreshes the hooks. A file planr cannot parse as a JSON object is left untouched with a note in the install output — add the snippet manually in that case.
- **Default, not mandatory.** `--no-hooks` skips hook installation entirely; deleting the planr entries from the hook files removes the behavior with no other effect.

## Codex trust model

Codex requires reviewing hooks before they run: after installing, open `/hooks` in the Codex TUI once and trust the planr entries. Codex records trust against the hook definition's hash, so if a future planr version changes the hook commands, Codex will ask again — that is the host working as intended.

## Removing hooks

Delete the planr entries from `.cursor/hooks.json` / `.claude/settings.json` / `.codex/hooks.json` (and `.cursor/hooks/planr-evidence-guard.sh`). Nothing else depends on them.

See also: [Model Routing](MODEL_ROUTING.md), [Goals](GOALS.md), [CLI Reference](CLI_REFERENCE.md).
