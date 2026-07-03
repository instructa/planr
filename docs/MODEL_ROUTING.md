# Model Routing

Declare which agent and model each kind of work should run on — once, in one file — and Planr hands the recommendation to whoever picks the work.

The pattern this makes declarative: strongest model plans and judges, a cheap fast steerable model implements, token-hungry side work (browser verification, codebase analysis) goes to budget profiles. Without a registry that knowledge lives in CLAUDE.md prose, Codex agent TOMLs, and Cursor frontmatter — three dialects that drift. With a registry it travels inside every pick packet.

Routing is **advisory by design**: Planr never calls model providers and never blocks a pick because a profile is unavailable. Your host (Codex, Claude Code, Cursor, any MCP client) stays the dispatch authority.

## Quick Start

Create `.planr/agents.toml` in your repo:

```toml
[profiles.fable-driver]
client = "cursor"
model = "fable-5"
effort = "high"
cost_tier = "premium"
capabilities = ["orchestration", "review", "planning"]
notes = "Planner/architect and judge. Verdicts stay on this tier."

[profiles.gpt55-coder]
client = "codex"
model = "gpt-5.5"
effort = "xhigh"
cost_tier = "standard"
capabilities = ["code", "steerable"]
notes = "Primary implementer: strong, fast, cheap on subscription."

[[routes]]
match = { work_type = "code" }
profile = "gpt55-coder"
fallbacks = ["fable-driver"]

[[routes]]
match = { work_type = "review" }
profile = "fable-driver"

[route_default]
profile = "gpt55-coder"
fallbacks = ["fable-driver"]
```

Validate and inspect:

```bash
planr agents check   # non-zero exit only on parse failure; warnings pass
planr agents list    # resolved profiles, routes, and warnings
```

From then on, every pick carries the recommendation:

```bash
planr pick --json
```

```json
"routing": {
  "profile": "gpt55-coder",
  "client": "codex",
  "model": "gpt-5.5",
  "effort": "xhigh",
  "cost_tier": "standard",
  "fallbacks": ["fable-driver"],
  "matched_selector": "work_type=code"
}
```

A driver session dispatches the right worker from the packet alone — and when the primary hits a rate limit, the fallback order is already in hand. No mid-run config edits.

## The Registry File

`.planr/agents.toml` has three parts:

- `[profiles.<id>]` — a named agent setting. `client` (which host dispatches it: `codex`, `claude-code`, `cursor`, `generic-mcp`) and `model` are required; `effort`, `cost_tier` (`premium` | `standard` | `budget`), `capabilities`, and `notes` are optional. Model ids and aliases pass through verbatim — Planr does not validate them against provider catalogs, so new models need no Planr release.
- `[[routes]]` — `match` selects work (`work_type = "code"` or `plan = "pln-1234abcd"`), `profile` names the primary, `fallbacks` the ordered alternatives.
- `[route_default]` — catches everything no route matched.

Resolution precedence per item: **per-item override > `work_type` route > `plan` route > default**. Within a level, the first declared route wins. If a chain's primary profile id is unknown, the first known fallback is promoted; a chain with no known profiles falls through to the next precedence level, so a typo never swallows lower routes. `matched_selector` in the output tells you which rule fired (`override`, `work_type=<v>`, `plan=<v>`, or `default`).

## Per-Item Overrides

When one item needs a different setting than its policy route — a gnarly refactor that deserves the premium tier, a bulk doc pass that can run on budget — pin it:

```bash
planr item route <item-id>                    # resolved route + source: override or policy
planr item route <item-id> --set fable-driver # pin; validates the profile id, emits route_overridden
planr item route <item-id> --clear            # unpin; policy applies again, emits route_override_cleared
```

The pin beats every policy route and shows up in the pick packet with `"matched_selector": "override"`. Overrides are repair-friendly: `--set` rejects a profile id the registry does not declare (when the registry is missing or malformed it warns and stores anyway, so offline edits stay possible), and a pin whose profile is later deleted from the registry is never an error — policy routing takes over and `item route` prints a repair hint. Both mutations are recorded as graph events, so `planr event list --item <id>` shows who re-routed what, when.

Tier the roles, not just the models: workers run safely on cheaper tiers because the pick packet bounds their scope, while review verdicts should stay on the strongest tier — `agents check` warns when review work routes to a `budget` profile. Background: [Cost Tiering](GOALS.md#cost-tiering).

## Host-Native Rendering

Routes only matter if the host actually dispatches the declared model, so `planr install codex|claude|cursor` closes the gap: when a registry is present, the provisioned subagent role files are rendered with pins taken from it instead of the shipped static defaults. The `work_type=code` route pins the worker role, the `work_type=review` route pins the reviewer role, and each render uses the host's exact vocabulary — Codex TOML gets `model` and `model_reasoning_effort` (with `developer_instructions` always present, since Codex silently ignores a role file without it), Claude frontmatter gets `model:` and `effort:`, Cursor frontmatter gets `model:` only.

Two safety rules keep this predictable:

- **Client matching**: a role file only pins profiles whose `client` matches the install target, scanning the route's fallback chain for the first match. A review route pointing at a Cursor profile never writes a Cursor model id into a Codex TOML — that role keeps its static default instead.
- **Provision-once**: existing files are never overwritten. After editing the registry, re-render explicitly with `planr install <client> --force`. Rendered files start with a `# generated from .planr/agents.toml` header so you (and future audit tooling) can tell them from hand-maintained ones.

Without a registry, installs write the static role files byte-identically to previous releases.

## Prompt Routing

`planr prompt routing [--client codex|claude|cursor|all]` prints a paste-ready block for the driver session: the prioritization table (every route, profile, and fallback in precedence order), per-host dispatch guidance including the traps that silently defeat pins (Codex requires `fork_turns: "none"` and a session restart after re-rendering; the `CLAUDE_CODE_SUBAGENT_MODEL` env var preempts Claude frontmatter; Cursor plan mode, admin policy, and Max Mode override silently), and process-dispatch snippets (`codex exec`, `pi`, `opencode run`) for hosts without role files, pre-filled from the `work_type=code` route. `--json` carries the same content structured.

## Run Audit

Every host has a silent override path — the `CLAUDE_CODE_SUBAGENT_MODEL` env var, Cursor plan/admin/Max-Mode policy, Codex full-history forks, org allowlists — so a pin alone is not proof. The audit loop closes this: workers report the profile they actually ran on via `planr log add`/`planr done --profile <id>` (or the `PLANR_PROFILE` env var, which rendered role files can export), the profile lands on the recorded run, and when it differs from the item's declared route Planr emits an advisory `route_mismatch_observed` event with the declared and actual ids.

- `planr trace item <id>` shows the declared route next to every run's actual client/profile with a `mismatch` marker.
- `planr doctor` reports the registry state (absent, degraded with parse context, loaded with counts and warnings) and flags rendered role files that drifted from the current registry (`planr install <client> --force` re-renders).
- `planr export`/`import` carry the registry with the package, preview-first; an existing registry at the destination is never silently overwritten.

Everything here is advisory (ADR-001): mismatches never fail logging, reviews, or closes. No profile reported, no run recorded, or no registry means no comparison and no event.

## Failure Behavior

- **No registry file**: nothing changes. Pick packets simply have no `routing` key.
- **Malformed registry**: `planr agents check` fails with the parser's line context; everything else (`pick`, `map`, `install`) keeps working with routing omitted — installs fall back to the static role files.
- **Warnings** (unknown profile references, empty or duplicate selectors, budget-tier review routes, secret-like values) never block anything; `agents check` lists them and still exits zero.
- Never put credentials in the registry — it holds configuration strings only, and secret-like values are flagged.

## Current Scope

Shipped today: the registry, `planr agents list|check`, the `routing` block in `planr pick --json`, per-item overrides (`planr item route [--set|--clear]`), the matching MCP tools (`planr_agents_list`, `planr_item_route`, `planr_item_route_set`, `planr_item_route_clear`) with identical JSON shapes, registry-rendered role files on `planr install` (with `--force` re-render), `planr prompt routing`, run-profile auditing (`--profile`/`PLANR_PROFILE`, `route_mismatch_observed` events, the `trace item` routing section), `doctor` registry diagnostics with drift detection, and registry packaging in export/import.

Planned next (see the product plan under `.planr/plans/`): an `agents init` scaffold and the final docs pass.
