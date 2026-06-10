---
name: planr
description: Single entry point for all Planr work. Use for any product, feature, fix, status, review, or summary request when the user has not named a specific Planr skill. Routes to the right Planr skill from live map state so the user never has to remember skill names.
---

# Planr Router

You are the dispatcher. Do not improvise workflow prompts; route to one Planr skill and follow it exactly. Skills are the prompts.

## Read State First

Routing is decided by live state, not by guessing:

```bash
planr project show --json
planr map show --json
planr review list --open
planr approval list --open
```

If no project exists, initialize before anything else:

```bash
planr project init "Project Name" --client all
```

Initialization happens once per repository. If a project already exists, never re-init: new features, refactors, and fixes get their own feature-scoped plan (`planr plan new "Feature"`), and `planr map build --from <build-plan-id>` extends the existing map with new linked items.

## Routing Table

Evaluate top to bottom; pick the first row that matches both intent and state:

| Intent | State condition | Route |
| --- | --- | --- |
| "build until done", "loop", "finish this feature autonomously" | any | `planr-loop` |
| status, "what's left", "what's blocked" | any | `planr-status` |
| summary of completed scope | any | `planr-summary` |
| new idea, PRD, scope, architecture | no plan, or plan needs refinement | `planr-plan` |
| new feature, refactor, or fix on an existing project | project exists, scope has no plan yet | `planr-plan` (feature-scoped plan, then extend the map) |
| plan exists but no map / map needs structure, dependencies, breakdown | build plan checked, map missing or stale | `planr-task-graph` |
| review requested or open review exists | open review on the map | `planr-review` |
| implement, fix, continue work | ready items exist on the map | `planr-work` |
| implement, but nothing is ready | all items blocked | `planr-status`, then report blockers to the user |

## Rules

- Route to exactly one skill per dispatch. If the request spans stages (idea -> running feature), route to `planr-loop` and let the loop sequence the stages.
- Never skip a stage: no map items without a checked build plan, no closure without review, no review verdict without log evidence.
- When delegating to a subagent, prompt it with a skill reference plus item id (for example: `Use $planr-work on item <id>`), never with a hand-written workflow prompt.
- The maker never reviews its own work. Reviews run through `planr-review` in a separate agent or subagent whenever the host supports it.
- If intent is genuinely ambiguous and state allows multiple routes, ask the user one short question instead of guessing.
