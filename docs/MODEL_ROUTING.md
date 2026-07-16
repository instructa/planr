# Model Routing

Declare which agent and model each kind of work should run on — once, in one file — and Planr hands the recommendation to whoever picks the work.

The pattern this makes declarative: strongest model plans and judges, a cheap fast steerable model implements, token-hungry side work (browser verification, codebase analysis) goes to budget profiles. Without a registry that knowledge lives in CLAUDE.md prose, Codex agent TOMLs, and Cursor frontmatter — three dialects that drift. With a registry it travels inside every pick packet.

Routing is **advisory by design**: Planr never calls model providers and never blocks a pick because a profile is unavailable. Your host (Codex, Claude Code, Cursor, any MCP client) stays the dispatch authority.

Want the whole flow on a concrete project first? [Worked Example: Routing a Small Web App](EXAMPLE_WEBAPP.md) walks a frontend/backend todo app from pool declaration to the audit trail, with real outputs.

## Quick Start

Apply the built-in native Codex binding to generate the registry, repository role TOMLs, and dispatch skill as one previewed unit:

```bash
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --preview --json
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --confirm --json
```

The generated `.planr/agents.toml` has the canonical direct routes:

```toml
[profiles.codex-sol-medium]
client = "codex"
model = "gpt-5.6-sol"
effort = "medium"
cost_tier = "standard"

[profiles.codex-terra-high]
client = "codex"
model = "gpt-5.6-terra"
effort = "high"
cost_tier = "standard"
skill = "planr-work"

[profiles.codex-sol-high]
client = "codex"
model = "gpt-5.6-sol"
effort = "high"
cost_tier = "premium"
skill = "planr-review"

[[routes]]
match = { work_type = "code" }
profile = "codex-terra-high"

[[routes]]
match = { work_type = "review" }
profile = "codex-sol-high"

[route_default]
profile = "codex-sol-medium"
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
  "profile": "codex-terra-high",
  "client": "codex",
  "model": "gpt-5.6-terra",
  "effort": "high",
  "cost_tier": "standard",
  "skill": "planr-work",
  "fallbacks": [],
  "matched_selector": "work_type=code"
}
```

The driver follows `.codex/skills/planr-native-routing/SKILL.md` and dispatches `agent_type: "planr-terra-high"` with `fork_turns: "none"`; `.codex/agents/planr-terra-high.toml` alone owns model and effort. Canonical Codex routes have no fallback chain and require no mid-run edits.

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

Routes only matter if the host actually dispatches the declared model. For Codex, `agents preset apply` generates the complete repository-owned native topology: Sol Medium driver, Terra Medium explorer, Terra High worker, Luna xHigh mechanical worker, Sol High reviewer, and explicit-only Sol Ultra moonshot planner. Each `.codex/agents/*.toml` file owns its exact model and effort, while `.codex/skills/planr-native-routing/SKILL.md` owns `agent_type` plus bounded `fork_turns` dispatch. Claude and Cursor retain their independent role-file renderers and model vocabularies.

Two safety rules keep this predictable:

- **One owner**: Codex call sites never repeat model or effort; only the selected repository role TOML contains them.
- **Preview and no-overwrite**: preset application previews every target and refuses different existing files. There is no force or global-config path.

Without an applied Codex preset, Planr does not invent or fall back to the removed Codex topology.

## Prompt Routing

`planr prompt routing [--client codex|claude|cursor|all]` prints a paste-ready block for the driver session: the prioritization table (every route, profile, and fallback in precedence order), native Codex dispatch through the generated repository role TOMLs with explicit `agent_type` and `fork_turns`, host-specific guidance for Claude and Cursor, and process-dispatch snippets (`pi`, `opencode run`) only for hosts without role files. Codex model and effort never appear at the call site; the selected `.codex/agents/*.toml` file is their sole owner. `--json` carries the same content structured.

## Run Audit

Host policy can still affect effective routing — for example the `CLAUDE_CODE_SUBAGENT_MODEL` environment variable, Cursor plan/admin/Max-Mode policy, Codex backend availability, and org allowlists — so a role pin alone is not proof. Native Codex rejects an all-history fork when the selected `agent_type` owns model or effort overrides. The audit loop closes this at two levels: workers report the profile they actually ran on via `planr log add`/`planr done --profile <id>` (or `PLANR_PROFILE`) for mismatch checks, and can attach a strict `--route-audit <observation.json>` that keeps requested, host-resolved, and effective model/effort/fork values separate. Every dimension carries enforcement confidence and a constrained evidence source; missing host evidence stays unavailable instead of inheriting the request.

- `planr trace item <id>` (MCP: `planr_trace_item`) shows the declared route next to every run's actual client/profile and three-stage observation with a `mismatch` marker.
- Runs also record the host they observably executed under (`observed_client`, detected from environment variables the hosts set themselves — no flags); a run whose host differs from the declared route's client emits an advisory `client_mismatch_observed` event, which catches exactly the deviation profile self-report cannot: a different host standing in for the declared client, even when the model matched.
- `planr doctor` reports the registry state (absent, degraded with parse context, loaded with counts and warnings) and flags rendered role files that drifted from the current registry (`planr install <client> --force` re-renders).
- `planr export`/`import` carry the registry with the package, preview-first; an existing registry at the destination is never silently overwritten.

Everything here is advisory (ADR-001): mismatches never fail logging, reviews, or closes. No profile reported, no run recorded, or no registry means no comparison and no event.

One legitimate mismatch source to know: a driver adding a live-verification log to a routed item runs on the driver profile by design, which emits a `route_mismatch_observed` event. The payload carries `log_kind`, so audit consumers can discount `verification` entries and alarm only on `completion` mismatches.

For single-host pools (e.g. all-Cursor), declare the host's *exact* model slugs (`claude-opus-4-8-thinking-high`, not `opus`): dispatch APIs resolve slugs, not aliases, and a driver forced to map `fable-5` onto the nearest slug at dispatch time is a silent translation the audit cannot see.

## Failure Behavior

- **No registry file**: nothing changes. Pick packets simply have no `routing` key.
- **Malformed registry**: `planr agents check` fails with the parser's line context; picks omit routing until the registry is repaired. Codex does not fall back to a removed static topology.
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
model = "gpt-5.6-terra"
effort = "high"
cost_tier = "standard"
skill = "planr-work"

[[routes]]
match = { work_type = "frontend" }
profile = "designer"

[[routes]]
match = { work_type = "design" }
profile = "designer"

[[routes]]
match = { work_type = "backend" }
profile = "backender"
```

Create items with the use-case work type (`planr item create ... --work-type frontend`) — or retag existing ones with `planr item update <id> --work-type frontend`, which is how planning agents tag `map build` output against the declared routes (the planning skills read `agents list` and do this without user involvement) — and the pick packet carries the full pairing — `"profile": "designer"`, `"model": "opus"`, `"skill": "frontend-design"` — so the driver dispatches profile and skill together (`Use $frontend-design on item <id>` on the profile's client/model). Workers pull their slice of the pool with `planr pick --work-type frontend`. `skill` is passthrough vocabulary like model ids: Planr never validates it against installed skills, and profiles without one omit the key entirely. A profile that needs different skills for different use cases is simply two profiles.

Declare the `client` you will actually dispatch on. A loop running inside one host dispatches that host's native subagents. A `client = "codex"` profile is honest only when the generated Codex `agent_type` role is dispatched; a Cursor-hosted model remains `client = "cursor"` even when its model family matches. This matters because workers report the profile id and effective-route evidence records the actual host.

Use more than one client only when the selected binding explicitly supports the cross-host topology. The built-in `mixed-host` binding keeps its Cursor Fable driver and dispatches repository-owned Codex Terra/Luna/Sol roles by `agent_type`; it does not synthesize a second process-level Codex model owner.

## Host Matrix

Where each host reads its model configuration from, and what silently defeats a pin there (state of July 2026):

| Host | Native mechanism | Rendered by `planr install`? | Silent overrides / traps |
| --- | --- | --- | --- |
| Cursor | `.cursor/agents/*.md` frontmatter `model: <id>` (default `inherit`) | yes (`cursor`) | Team-admin model policy, plan availability, and Max-Mode-only models override without error; legacy request-based plans force Composer for subagents; subagent transcripts record no model field, so the actual model cannot be verified from artifacts after the fact — the dispatch parameters in the driver session are the only record |
| Claude Code | `planr-worker.md`/`planr-reviewer.md` frontmatter `model:` + `effort:` | yes (`claude`) | `CLAUDE_CODE_SUBAGENT_MODEL` clamps frontmatter and per-invocation models with no signal ([#57718](https://github.com/anthropics/claude-code/issues/57718)); since v2.1.196 `inherit` behaves as unset; org `availableModels` allowlists fall back silently |
| Codex CLI | `.codex/agents/*.toml` with `model` + `model_reasoning_effort` | yes (`codex`) | Native Codex rejects `fork_turns = "all"` when `agent_type` selects a role with model or effort overrides; use `fork_turns = "none"` or an evidenced positive partial fork, and restart after applying changed role files because the registry loads at session start ([#26408](https://github.com/openai/codex/issues/26408)) |
| opencode | `opencode.json` `agent.<name>.model = "provider/model-id"` or `.opencode/agents/*.md` frontmatter | no — use the `planr prompt routing` process snippet | Subagent inherits the primary model when unset; malformed `provider/model-id` strings (quoting, trailing newline) raise `ProviderModelNotFoundError` ([#5623](https://github.com/sst/opencode/issues/5623)) |
| Pi | none by design — process-level dispatch (`pi --provider --model --thinking`) or the `pi-subagents` extension (`.pi/agents/*.md`) | no — use the `planr prompt routing` process snippet | Extension model-scope enforcement against `enabledModels` is opt-in; without it, pins are best-effort |

For the hosts without rendered role files, `planr prompt routing` prints ready process-dispatch snippets pre-filled from the registry. Whatever the host does, the [run audit](#run-audit) catches silent overrides after the fact.

## Command Summary

The registry surface end to end: `planr agents init [--force]` atomically creates the native Codex registry, matching repository role TOMLs, and routing skill, while flags and the wizard build non-Codex pools. `planr agents list|check` inspect and validate, `planr pick --json` carries the routing block, and per-item pins override it. Verified preset apply reuses the same canonical owner, so its registry and role preview remains conflict-free after init. `planr install claude|cursor [--force]` retains independent host rendering.
