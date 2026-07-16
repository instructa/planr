# Worked Example: Routing a Small Web App

A complete, replayable walkthrough of [model routing](MODEL_ROUTING.md) on a real shape of work: a todo web app with a frontend and a backend, implemented by different models. Every output below is from an actual run — copy the commands into an empty directory and you get the same results.

The pool we want:

| Use case | Who runs it | Paired skill |
| --- | --- | --- |
| planning + synthesis | Sol Medium | generated driver role |
| frontend mechanical implementation | Luna xHigh | `planr-work` |
| backend implementation | Terra High | `planr-work` |
| independent review | Sol High | `planr-review` |

## Step 1: Project and pool — two commands, once

```bash
planr project init "Todo Webapp"

planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --preview --json
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --confirm --json

planr agents check
```

```text
applied balanced + codex-openai
generated .planr/agents.toml, .codex/agents/*.toml, and .codex/skills/planr-native-routing/SKILL.md
```

Preview lists every repository-local artifact before confirmation. The canonical routes directly select one native role per work type and contain no fallback chain.

## Step 2: Items carry the use case as their work type

Work types are free-form, so the use case is just `--work-type`:

```bash
planr item create "API: task CRUD endpoints" \
  --description "Express routes GET/POST/PATCH/DELETE /api/tasks with sqlite store" \
  --work-type code

planr item create "UI: task list and add form" \
  --description "React list view + optimistic add form against /api/tasks" \
  --work-type mechanical

planr item create "UI: design pass on empty and error states" \
  --description "Empty state, loading skeleton, error toast" \
  --work-type exploration
```

## Step 3: The pick decides who works

Same board, two different answers. The backend pull:

```bash
planr pick --work-type code --json
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

And the frontend pull:

```bash
planr pick --work-type mechanical --json
```

```json
"routing": {
  "profile": "codex-luna-xhigh",
  "client": "codex",
  "model": "gpt-5.6-luna",
  "effort": "xhigh",
  "skill": "planr-work",
  "fallbacks": [],
  "matched_selector": "work_type=mechanical"
}
```

The driver follows the generated routing skill: backend code dispatches `agent_type: "planr-terra-high"`, while bounded frontend work dispatches `agent_type: "planr-luna-xhigh"`; both calls use `fork_turns: "none"`. Their role TOMLs alone own model and effort. Two workers can pull in parallel, and the pick lease keeps one owner per item.

## Step 4: Workers close with evidence and their profile

```bash
planr done <item-id> \
  --summary "CRUD endpoints implemented; 8 supertest cases green" \
  --files server/routes/tasks.ts \
  --tests "vitest --run: 8 passed" \
  --profile codex-terra-high
```

`--profile` (or the `PLANR_PROFILE` env var) records which registry profile the run actually executed on. It is part of the evidence, not a formality — the next step is why.

## Step 5: The trace proves the plan was followed

Every host has a silent override path (env clamps, admin policies, fork semantics — see the [host matrix](MODEL_ROUTING.md#host-matrix)), so a pin alone is not proof. In this run, the backend item was closed on the declared profile and the frontend item deliberately on the wrong one:

```bash
planr trace item <backend-item>
planr trace item <frontend-item>
```

```text
routing declared: codex-terra-high (work_type=code)
  run run-5552fad6 profile codex-terra-high

routing declared: codex-luna-xhigh (work_type=mechanical)
  run run-bc0994cb profile codex-sol-medium (differs from declared route; advisory)
```

The mismatch also lands as a `route_mismatch_observed` event (`planr event list`). Advisory by design: nothing blocked, but the drift is on record instead of invisible.

## The whole model in one line each

1. **Declare** the pool once: `planr agents init` atomically creates the native Codex registry and repository roles; flags and the wizard create non-Codex pools.
2. **Tag** items with their use case: `--work-type code|mechanical|exploration|review|moonshot`.
3. **Pick** routes: every packet carries the direct profile, model, skill, and an empty fallback list.
4. **Deliver** with `--profile` as part of the evidence.
5. **Trace** proves declared vs. actual — silent overrides get caught.

One-off exceptions never need a registry edit: `planr item route <id> --set codex-sol-ultra` explicitly pins one moonshot planning item and `--clear` restores policy. Details and failure behavior: [Model Routing](MODEL_ROUTING.md); the cost logic behind the tiers: [GOALS.md Cost Tiering](GOALS.md#cost-tiering).
