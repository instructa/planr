# Model Routing

Declare which agent and model each kind of work should run on — once, in one file — and Planr hands the recommendation to whoever picks the work.

The pattern this makes declarative: strongest model plans and judges, a cheap fast steerable model implements, token-hungry side work (browser verification, codebase analysis) goes to budget profiles. Without a registry that knowledge lives in CLAUDE.md prose, Codex agent TOMLs, and Cursor frontmatter — three dialects that drift. With a registry it travels inside every pick packet.

Routing is **advisory by design**: Planr never calls model providers and never blocks a pick because a profile is unavailable. Your host (Codex, Claude Code, Cursor, any MCP client) stays the dispatch authority.

Want the whole flow on a concrete project first? [Worked Example: Routing a Small Web App](EXAMPLE_WEBAPP.md) walks a frontend/backend todo app from pool declaration to the audit trail, with real outputs.

## Quick Start

One command writes a working starter registry — the cost-tiering defaults with a premium driver, a standard implementer, and a budget helper, commented so the tiers explain themselves:

```bash
planr agents init          # writes .planr/agents.toml; never overwrites without --force
```

Or declare `.planr/agents.toml` by hand:

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

Every host has a silent override path — the `CLAUDE_CODE_SUBAGENT_MODEL` env var, Cursor plan/admin/Max-Mode policy, Codex full-history forks, org allowlists — so a pin alone is not proof. The audit loop closes this at two levels: workers report the profile they actually ran on via `planr log add`/`planr done --profile <id>` (or `PLANR_PROFILE`) for backward-compatible mismatch checks, and can attach a strict `--route-audit <observation.json>` that keeps requested, host-resolved, and effective model/effort/fork values separate. Every dimension carries enforcement confidence and a constrained evidence source; missing host evidence stays unavailable instead of inheriting the request.

- `planr trace item <id>` (MCP: `planr_trace_item`) shows the declared route next to every run's actual client/profile and three-stage observation with a `mismatch` marker.
- Runs also record the host they observably executed under (`observed_client`, detected from environment variables the hosts set themselves — no flags); a run whose host differs from the declared route's client emits an advisory `client_mismatch_observed` event, which catches exactly the deviation profile self-report cannot: a different host standing in for the declared client, even when the model matched.
- `planr doctor` reports the registry state (absent, degraded with parse context, loaded with counts and warnings) and flags rendered role files that drifted from the current registry (`planr install <client> --force` re-renders).
- `planr export`/`import` carry the registry with the package, preview-first; an existing registry at the destination is never silently overwritten.

Everything here is advisory (ADR-001): mismatches never fail logging, reviews, or closes. No profile reported, no run recorded, or no registry means no comparison and no event.

One legitimate mismatch source to know: a driver adding a live-verification log to a routed item runs on the driver profile by design, which emits a `route_mismatch_observed` event. The payload carries `log_kind`, so audit consumers can discount `verification` entries and alarm only on `completion` mismatches.

For single-host pools (e.g. all-Cursor), declare the host's *exact* model slugs (`claude-opus-4-8-thinking-high`, not `opus`): dispatch APIs resolve slugs, not aliases, and a driver forced to map `fable-5` onto the nearest slug at dispatch time is a silent translation the audit cannot see.

## Failure Behavior

- **No registry file**: nothing changes. Pick packets simply have no `routing` key.
- **Malformed registry**: `planr agents check` fails with the parser's line context; everything else (`pick`, `map`, `install`) keeps working with routing omitted — installs fall back to the static role files.
- **Warnings** (unknown profile references, empty or duplicate selectors, budget-tier review routes, secret-like values) never block anything; `agents check` lists them and still exits zero.
- Never put credentials in the registry — it holds configuration strings only, and secret-like values are flagged.

## Use-Case Pools

Work types are free-form, and that makes them the use-case dimension: beyond the built-in vocabulary (`code`, `fix`, `review`, `docs`, ...), any string you pass to `--work-type` routes. Combined with per-profile skill pairing, the registry becomes a small agent pool — each use case names who runs it, on what model, with which skill:

```toml
[profiles.designer]
client = "claude-code"
model = "opus"
effort = "high"
cost_tier = "premium"
skill = "frontend-design"     # dispatch this profile *with* this skill

[profiles.backender]
client = "codex"
model = "gpt-5.5"
effort = "xhigh"
cost_tier = "standard"
skill = "planr-work"

[[routes]]
match = { work_type = "frontend" }
profile = "designer"
fallbacks = ["driver"]

[[routes]]
match = { work_type = "design" }
profile = "designer"

[[routes]]
match = { work_type = "backend" }
profile = "backender"
fallbacks = ["driver"]
```

Create items with the use-case work type (`planr item create ... --work-type frontend`) — or retag existing ones with `planr item update <id> --work-type frontend`, which is how planning agents tag `map build` output against the declared routes (the planning skills read `agents list` and do this without user involvement) — and the pick packet carries the full pairing — `"profile": "designer"`, `"model": "opus"`, `"skill": "frontend-design"` — so the driver dispatches profile and skill together (`Use $frontend-design on item <id>` on the profile's client/model). Workers pull their slice of the pool with `planr pick --work-type frontend`. `skill` is passthrough vocabulary like model ids: Planr never validates it against installed skills, and profiles without one omit the key entirely. A profile that needs different skills for different use cases is simply two profiles.

Declare the `client` you will actually dispatch on. A loop running inside one host dispatches that host's subagents — an in-Cursor driver that dispatches Cursor subagents with per-dispatch models is running `client = "cursor"` profiles in practice, even when the model matches. A `client = "codex"` profile is only honest when the driver really spawns a Codex process (`codex exec ...`). This matters for the audit: workers report the *profile id*, so a profile whose declared client differs from the real dispatch host passes the mismatch check on the model alone — the client deviation stays invisible.

When do you actually need more than one client? Hosts with a full model catalog (Cursor) can serve an entire pool natively — an all-`cursor` registry with different models per profile is the normal case there. Cross-client profiles exist for two real situations: vendor-locked hosts (Claude Code dispatches only Anthropic models, Codex CLI only OpenAI models — a Claude-Code driver that wants a GPT implementer must process-dispatch via `codex exec`), and subscription economics (the same model can bill differently per host, so routing backend work through a flat-rate CLI subscription instead of the driver host's quota is a legitimate cost decision).

## Host Matrix

Where each host reads its model configuration from, and what silently defeats a pin there (state of July 2026):

| Host | Native mechanism | Rendered by `planr install`? | Silent overrides / traps |
| --- | --- | --- | --- |
| Cursor | `.cursor/agents/*.md` frontmatter `model: <id>` (default `inherit`) | yes (`cursor`) | Team-admin model policy, plan availability, and Max-Mode-only models override without error; legacy request-based plans force Composer for subagents; subagent transcripts record no model field, so the actual model cannot be verified from artifacts after the fact — the dispatch parameters in the driver session are the only record |
| Claude Code | `planr-worker.md`/`planr-reviewer.md` frontmatter `model:` + `effort:` | yes (`claude`) | `CLAUDE_CODE_SUBAGENT_MODEL` clamps frontmatter and per-invocation models with no signal ([#57718](https://github.com/anthropics/claude-code/issues/57718)); since v2.1.196 `inherit` behaves as unset; org `availableModels` allowlists fall back silently |
| Codex CLI | `.codex/agents/*.toml` with `model` + `model_reasoning_effort` | yes (`codex`) | `fork_turns = "all"` intentionally drops the child's `agent_type`/`model` — use `fork_turns = "none"` or a partial fork; the role registry loads at session start ([#26408](https://github.com/openai/codex/issues/26408)), so re-renders need a restart |
| opencode | `opencode.json` `agent.<name>.model = "provider/model-id"` or `.opencode/agents/*.md` frontmatter | no — use the `planr prompt routing` process snippet | Subagent inherits the primary model when unset; malformed `provider/model-id` strings (quoting, trailing newline) raise `ProviderModelNotFoundError` ([#5623](https://github.com/sst/opencode/issues/5623)) |
| Pi | none by design — process-level dispatch (`pi --provider --model --thinking`) or the `pi-subagents` extension (`.pi/agents/*.md`) | no — use the `planr prompt routing` process snippet | Extension model-scope enforcement against `enabledModels` is opt-in; without it, pins are best-effort |

For the hosts without rendered role files, `planr prompt routing` prints ready process-dispatch snippets pre-filled from the registry. Whatever the host does, the [run audit](#run-audit) catches silent overrides after the fact.

## Command Summary

The registry surface end to end: `planr agents init [--force]` scaffolds, `planr agents list|check` inspect and validate, `planr pick --json` carries the `routing` block, `planr item route [--set|--clear]` pins per item, the MCP tools (`planr_agents_list`, `planr_item_route`, `planr_item_route_set`, `planr_item_route_clear`) return identical JSON shapes, `planr install <client> [--force]` renders host role files from the registry, `planr prompt routing` prints the driver dispatch block, `planr log add`/`done --profile` (or `PLANR_PROFILE`) feed the run audit, `planr trace item` and `planr doctor` surface mismatches and drift, and `planr export`/`import` carry the registry preview-first.
