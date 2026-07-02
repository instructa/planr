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

## Failure Behavior

- **No registry file**: nothing changes. Pick packets simply have no `routing` key.
- **Malformed registry**: `planr agents check` fails with the parser's line context; everything else (`pick`, `map`, `install`) keeps working with routing omitted.
- **Warnings** (unknown profile references, empty or duplicate selectors, budget-tier review routes, secret-like values) never block anything; `agents check` lists them and still exits zero.
- Never put credentials in the registry — it holds configuration strings only, and secret-like values are flagged.

## Current Scope

Shipped today: the registry, `planr agents list|check`, the `routing` block in `planr pick --json`, per-item overrides (`planr item route [--set|--clear]`), and the matching MCP tools (`planr_agents_list`, `planr_item_route`, `planr_item_route_set`, `planr_item_route_clear`) with identical JSON shapes.

Planned next (see the product plan under `.planr/plans/`): rendering host role files (Codex TOML, Claude/Cursor agent frontmatter) from the registry, declared-vs-actual profile auditing on runs in `planr trace item`, and an `agents init` scaffold.

Until host-file rendering lands, keep pinning worker tiers in the provisioned role files as documented in [Cost Tiering](GOALS.md#cost-tiering) — the registry is the source of truth for routing decisions, the role files still carry the host-native pins.
