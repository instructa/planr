# Host Hooks

`planr install codex|claude|cursor` wires Planr into the host's native hook system **by default** — skip it with `--no-hooks`. The hooks solve one problem: loop state must survive the moments context is created or destroyed. Every new session and every context compaction gets the map state injected automatically via `planr prime`, so recovery stops being discipline and becomes mechanism.

## What gets installed

| Host | File | Events | Runs |
| --- | --- | --- | --- |
| Cursor | `.cursor/hooks.json` | `sessionStart`, `preCompact` | `planr prime` |
| | | `subagentStop` | `.cursor/hooks/planr-evidence-guard.sh` |
| Claude Code | `.claude/settings.json` | `SessionStart` (matcher `startup\|resume\|compact`), `PreCompact` | `planr prime --hook-json` |
| Codex CLI | `.codex/hooks.json` | `SessionStart`, `PostCompact` | `planr prime` |

`planr prime` prints one compact state block (project, map counts, your held items with log status, the goal contract, registry presence, and the next command) — well under ~1k tokens, deterministic, read-only. `--hook-json` wraps it in the Claude Code SessionStart envelope (`hookSpecificOutput.additionalContext`).

The evidence guard is advisory: a Cursor subagent that stops while a pick has no completion log gets one follow-up message naming the item and the two ways out (`planr done ...` or `planr pick release ...`). It never blocks.

## Design rules

- **Fail open, always.** Every hook command ends in `|| true` with a 10-second timeout; a missing, broken, or uninitialized planr never blocks a session. In a repo without a Planr database, `planr prime` exits silently and creates nothing.
- **Additive merge, never overwrite.** Existing hook files keep their entries; planr adds its own only if absent (re-installs are no-ops). A file planr cannot parse as a JSON object is left untouched with a note in the install output — add the snippet manually in that case.
- **Default, not mandatory.** `--no-hooks` skips hook installation entirely; deleting the planr entries from the hook files removes the behavior with no other effect.

## Codex trust model

Codex requires reviewing hooks before they run: after installing, open `/hooks` in the Codex TUI once and trust the planr entries. Codex records trust against the hook definition's hash, so if a future planr version changes the hook commands, Codex will ask again — that is the host working as intended.

## Removing hooks

Delete the planr entries from `.cursor/hooks.json` / `.claude/settings.json` / `.codex/hooks.json` (and `.cursor/hooks/planr-evidence-guard.sh`). Nothing else depends on them.

See also: [Model Routing](MODEL_ROUTING.md), [Goals](GOALS.md), [CLI Reference](CLI_REFERENCE.md).
