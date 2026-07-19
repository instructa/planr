# Worked Example: Routing a Small Web App

This provider-neutral example routes a todo web app by work type. Planr Core treats every profile field as opaque data and does not select a host or model.

## 1. Create the project and registry

```bash
planr project init "Todo Webapp"
planr agents init
```

Edit the repository-local `.planr/agents.toml` with profiles owned by your team or routing package:

```toml
[profiles.frontend-worker]
client = "your-host"
model = "your-fast-model"
effort = "medium"
cost_tier = "budget"
skill = "planr-work"

[profiles.backend-worker]
client = "your-host"
model = "your-engineering-model"
effort = "high"
cost_tier = "standard"
skill = "planr-work"

[profiles.independent-reviewer]
client = "your-host"
model = "your-review-model"
effort = "high"
cost_tier = "premium"
skill = "planr-review"

[[routes]]
match = { work_type = "frontend" }
profile = "frontend-worker"

[[routes]]
match = { work_type = "backend" }
profile = "backend-worker"

[[routes]]
match = { work_type = "review" }
profile = "independent-reviewer"
```

Validate without contacting a provider:

```bash
planr agents check
planr agents list
```

Alternatively, an external routing tool such as [Switchloom v0.2.1](https://github.com/instructa/switchloom/releases/tag/v0.2.1) can manage repository-local declarations and host roles outside Planr. After that external lifecycle step, Planr still sees only provider-neutral declarations:

```bash
planr agents check
planr agents list --json
```

## 2. Tag items by use case

```bash
planr item create "API: task CRUD endpoints" \
  --description "GET/POST/PATCH/DELETE /api/tasks with sqlite store" \
  --work-type backend

planr item create "UI: task list and add form" \
  --description "List view and optimistic add form" \
  --work-type frontend
```

## 3. Pick and dispatch

```bash
planr pick --work-type backend --json
```

The pick packet contains the selected opaque profile, client, model, effort, skill, and selector. The host remains responsible for dispatching it; Planr never claims that the requested model actually ran.

## 4. Close with evidence

```bash
planr done <item-id> \
  --summary "CRUD endpoints implemented; 8 tests green" \
  --files server/routes/tasks.ts \
  --tests "vitest --run: 8 passed" \
  --profile backend-worker
```

`--profile` records the run's claimed profile. For stronger evidence, attach a route observation that keeps requested, host-resolved, and effective values separate.

## 5. Trace declared versus actual

```bash
planr trace item <item-id>
```

A mismatch is recorded as advisory evidence instead of silently rewritten. Missing effective-host evidence stays unavailable and is never inferred from the registry.

One-off exceptions use `planr item route <id> --set <profile>`; `--clear` restores policy resolution. See [Model Routing](MODEL_ROUTING.md) and [External Routing Declarations](ROUTING_BUNDLES.md).
