# Worked Example: Routing a Small Web App

A complete, replayable walkthrough of [model routing](MODEL_ROUTING.md) on a real shape of work: a todo web app with a frontend and a backend, implemented by different models. Every output below is from an actual run — copy the commands into an empty directory and you get the same results.

The pool we want:

| Use case | Who runs it | Paired skill |
| --- | --- | --- |
| planning + review | Fable on Cursor (the driver session) | — |
| frontend, design | Opus on Claude Code | `frontend-design` |
| backend | GPT-5.5 on Codex | `planr-work` |

## Step 1: Project and pool — two commands, once

```bash
planr project init "Todo Webapp"

planr agents init \
  --profile driver=cursor/fable-5@high#premium \
  --profile frontender=claude-code/opus@high#premium \
  --profile backender=codex/gpt-5.5@xhigh#standard \
  --skill frontender=frontend-design \
  --skill backender=planr-work \
  --route frontend=frontender,driver \
  --route backend=backender,driver \
  --route review=driver \
  --default-route backender,driver

planr agents check
```

```text
wrote .planr/agents.toml (generated from the flag specs)
agent registry check passed
```

Prefer questions over flags? `planr agents init --interactive` walks the same inputs as guided prompts and can render the host role files at the end. Either way, validation is fail-closed: a typo in a profile reference errors before anything is written.

Note what the routes encode: review work is pinned to the premium driver (verdicts stay strong), everything else runs on the cheaper tier its use case calls for, and every route falls back to the driver when the primary hits a rate limit.

## Step 2: Items carry the use case as their work type

Work types are free-form, so the use case is just `--work-type`:

```bash
planr item create "API: task CRUD endpoints" \
  --description "Express routes GET/POST/PATCH/DELETE /api/tasks with sqlite store" \
  --work-type backend

planr item create "UI: task list and add form" \
  --description "React list view + optimistic add form against /api/tasks" \
  --work-type frontend

planr item create "UI: design pass on empty and error states" \
  --description "Empty state, loading skeleton, error toast" \
  --work-type frontend
```

## Step 3: The pick decides who works

Same board, two different answers. The backend pull:

```bash
planr pick --work-type backend --json
```

```json
"routing": {
  "profile": "backender",
  "client": "codex",
  "model": "gpt-5.5",
  "effort": "xhigh",
  "cost_tier": "standard",
  "skill": "planr-work",
  "fallbacks": ["driver"],
  "matched_selector": "work_type=backend"
}
```

And the frontend pull:

```bash
planr pick --work-type frontend --json
```

```json
"routing": {
  "profile": "frontender",
  "client": "claude-code",
  "model": "opus",
  "skill": "frontend-design",
  "fallbacks": ["driver"],
  "matched_selector": "work_type=frontend"
}
```

The driver session dispatches from the packet alone: the backend item goes to Codex (`codex exec --model gpt-5.5 -c model_reasoning_effort="xhigh" ...` — `planr prompt routing` prints this snippet pre-filled), the frontend item to a Claude Code subagent dispatched with its paired skill (`Use $frontend-design on item <id>`). Two workers can pull in parallel — each `pick --work-type <use-case>` takes its own lane, and the pick lease keeps one owner per item.

## Step 4: Workers close with evidence and their profile

```bash
planr done <item-id> \
  --summary "CRUD endpoints implemented; 8 supertest cases green" \
  --files server/routes/tasks.ts \
  --tests "vitest --run: 8 passed" \
  --profile backender
```

`--profile` (or the `PLANR_PROFILE` env var) records which registry profile the run actually executed on. It is part of the evidence, not a formality — the next step is why.

## Step 5: The trace proves the plan was followed

Every host has a silent override path (env clamps, admin policies, fork semantics — see the [host matrix](MODEL_ROUTING.md#host-matrix)), so a pin alone is not proof. In this run, the backend item was closed on the declared profile and the frontend item deliberately on the wrong one:

```bash
planr trace item <backend-item>
planr trace item <frontend-item>
```

```text
routing declared: backender (work_type=backend)
  run run-5552fad6 profile backender

routing declared: frontender (work_type=frontend)
  run run-bc0994cb profile driver (differs from declared route; advisory)
```

The mismatch also lands as a `route_mismatch_observed` event (`planr event list`). Advisory by design: nothing blocked, but the drift is on record instead of invisible.

## The whole model in one line each

1. **Declare** the pool once: `planr agents init` (flags, wizard, or the plain scaffold).
2. **Tag** items with their use case: `--work-type frontend|backend|...` — free-form.
3. **Pick** routes: every packet carries profile, model, skill, and fallbacks.
4. **Deliver** with `--profile` as part of the evidence.
5. **Trace** proves declared vs. actual — silent overrides get caught.

One-off exceptions never need a registry edit: `planr item route <id> --set driver` pins a single gnarly item to the premium tier and `--clear` restores policy. Details and failure behavior: [Model Routing](MODEL_ROUTING.md); the cost logic behind the tiers: [GOALS.md Cost Tiering](GOALS.md#cost-tiering).
