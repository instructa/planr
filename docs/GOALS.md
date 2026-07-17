# Long-Running Goals

Planr makes long-running, autonomous goal runs durable. A loop driver — Codex `/goal`, Claude Code `/goal` or `/loop`, an automation, or a human re-dispatching a skill — supplies continuation pressure: "do not stop until the goal holds." Planr supplies everything such a run loses between sessions: the plan, the task map, picks, evidence logs, review gates, approvals, and recovery.

This is complementary, not competing. `/goal` stays the orchestrator; Planr is the state layer underneath it. Without a native loop primitive you lose only automatic re-prompting — never state, evidence, or recovery.

## Division Of Labor

| Concern | Owner |
| --- | --- |
| Continuation pressure, re-prompting, session autonomy | loop driver (`/goal`, automation, human) |
| Scope, acceptance criteria, verification contract | Planr plan (`planr plan new/check`) |
| Task state, dependencies, what is next | Planr map (`planr map`, `planr pick`) |
| Stop condition that survives compaction | Planr context (`--tag goal-contract`) |
| Proof the work happened and runs | Planr logs (`planr log`, `planr done`) |
| Maker/checker separation | Planr reviews (`planr review`) + subagent roles |
| Recovery after session loss or host switch | Planr map + stored contract, from zero chat context |

## The Workflow

### 1. Prep — `$planr-goal` (once, interactive)

```text
$planr-goal Add CSV export to the reports page, should work in the browser
```

The skill compiles the goal and stops — no implementation:

- creates and checks a feature-scoped plan (`planr plan new` -> `plan check`; strict, empty sections fail),
- builds the map and links execution order (`planr map build` is idempotent; `planr link add ... --type blocks`),
- stores the goal contract durably in Planr:

```bash
planr context add "GOAL CONTRACT pl-csv-export: DONE when every in-scope map item is closed with log evidence, all reviews closed with verdict complete, no open approvals in scope, and a live browser verification log exists for the export flow. Iteration budget: 10." --tag goal-contract
```

- prints the exact starter command for your host.

### 2. Execute — the loop driver runs `$planr-loop`

```text
/goal Use $planr-loop on plan pl-csv-export. The loop contract is stored in planr context (tag: goal-contract). Continue until the contract holds or the iteration budget is exhausted. You are operating autonomously: the user is not watching, so never end a turn on a plan, a question, or a promise — proceed until the contract holds or you are blocked on input only the user can provide.
```

The autonomy clause matters on long runs: deep into a session, frontier models occasionally end a turn with a statement of intent instead of the corresponding action, or pause to ask permission they already have. Stating the operating mode up front prevents both.

Each iteration follows the `$planr-loop` protocol:

```text
1. planr plan audit <plan-id>   one-call contract verdict; holds -> exit
2. $planr-work     pick exactly one ready item, implement, finish with planr done --review
3. live verify     run the platform verification, log it with planr log add --kind verification --cmd
4. $planr-review   independent audit; complete -> review close --close-target,
                   findings -> fix items become the next ready items
5. repeat
```

`plan audit` replaces the hand-rolled final audit: it checks `items_settled`, `reviews_complete`, `approvals_clear`, and `verification_logged` clause by clause with evidence, includes the stored goal contract, and answers `holds: true/false` in one command.

The per-item path is three commands since v1.1.6:

```bash
planr pick --json --plan <plan-id>                           # flat work packet, leased only from the goal's plan
planr done <item-id> --summary "..." --cmd "..." --review --next
planr review close <review-id> --verdict complete --reviewer <id> --close-target
```

`--plan` keeps the lease inside the goal contract: when several plans share the board (a parallel feature, leftovers from an aborted prep run), a plan-scoped goal run never picks work outside its own plan. A pick that finds nothing in scope never widens silently: it reports `reason: "nothing_ready"` when nothing is ready at all, or `reason: "ready_items_excluded_by_filter"` with the excluded items, the cause per item, and the exact `repair` pick commands when ready work exists outside the filter.

`done`/`close`/`review close` responses and the pick packet include a `remaining` snapshot (`counts` with explicit zeros for every status, `settled`, `total`), so the orchestrator evaluates the stop condition straight from the completion output — no extra `map status` round-trip. The same responses list what each settlement `unlocked`, so the loop sees its next work without re-reading the map. `--next` never hands a worker its own freshly created review, so maker and checker stay separate even in compact loops. The review verdict records `review_mode` (`single_agent` or `independent`) automatically from worker identity — no ceremony note needed. The contract's "all reviews closed" clause audits review items that exist; an item closed with plain `done` satisfies the contract without a review gate, so low-signal reviews can be skipped without blocking `plan audit`.

### 3. Finish

When the contract holds, the loop exits through `$planr-summary`: an evidence-backed account of what shipped, which commands proved it, and what (if anything) stayed blocked.

## Recovery

The defining property of a long-running goal: the session will die before the goal does. With Planr that costs nothing. Start a new session — same host or a different one — with the same starter line:

```text
/goal Use $planr-loop on plan pl-csv-export. The loop contract is stored in planr context (tag: goal-contract).
```

Iteration 1 reads the map and the stored contract: items already settled stay settled, open reviews stay open, the next ready item is picked. No chat history needed. `planr recover sweep` handles stale picks from interrupted workers.

## Per-Host Setup

### Codex with `/goal`

Install the plugin, initialize the repository, and optionally apply a repository routing bundle:

```bash
codex plugin marketplace add instructa/planr
codex plugin add planr@planr
planr project init "My Product"
planr-routing compile balanced --host codex-openai --output routing-bundle.json
planr routing bundle preview routing-bundle.json
planr routing bundle apply routing-bundle.json
```

Then:

```text
$planr-goal <your goal>          # prep: plan, map, contract, starter command
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract).
```

When a routing bundle is applied, the `/goal` PM follows its generated repository skill and roles. The bundle owner decides exact models, effort, role names, and fork policy; Planr only carries the resulting opaque route in pick packets. Codex Automations use the same starter line.

### Claude Code

Same shape via the plugin (`/plugin install planr@planr`), which registers the `planr-worker` and `planr-reviewer` subagents automatically:

```text
/planr:planr-goal <your goal>
/goal Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract).
```

`/loop` works for fixed-cadence runs instead of goal-conditioned ones. The registered worker subagent pins a cheaper model tier; see [Cost Tiering](#cost-tiering).

### Cursor

No `/goal` primitive, but full subagent support. Install once:

```bash
planr install cursor   # writes .cursor/mcp.json, .cursor/agents/planr-worker.md + planr-reviewer.md, and the skills
```

Then run the identical protocol with the human (or an automation) as the re-dispatcher:

```text
Use $planr-goal: <your goal>
Use $planr-loop on plan <plan-id>. The loop contract is stored in planr context (tag: goal-contract).
```

`$planr-loop` iterates within the session under its own budget, dispatching `/planr-worker` and `/planr-reviewer` per item — the provisioned subagents preload the work and review protocols, so dispatches stay one line. If the session ends before the contract holds, dispatch the same line again — recovery is identical to the `/goal` case. Parallelism, background subagents, and worktree caveats: [Cursor](CURSOR.md).

### Hosts without a loop primitive

Identical protocol; the human (or a background agent) is the re-dispatcher, and worker/reviewer run as separate sequential dispatches when the host has no subagents.

### Plain MCP clients

Any MCP-capable agent uses the same flow over `planr mcp`. Every session starts with map state, so the loop is resumable by construction.

## Cost Tiering

A goal run has distinct roles with different intelligence needs. Planr represents those decisions as opaque profiles and routes; it does not choose providers, models, effort levels, fallback chains, or host-native role names.

An optional routing-policy package can map planning, exploration, implementation, mechanical work, and independent review onto different cost or quality tiers. If a routing bundle generated repository roles, follow its generated instructions and verify effective child metadata because declarations alone are not proof. The official package and its host-specific caveats live under [`planr-routing`](../planr-routing/docs/MODEL_ROUTING_POLICY.md); the Core contract is documented in [Model Routing](MODEL_ROUTING.md).

## Coming From Other Goal Tools

If you already run goal workflows with other tools, the concepts map directly:

| Elsewhere | In Planr |
| --- | --- |
| Goal charter file (`goal.md`) | product/build plan (`planr plan new`, rich scope + verification) |
| Board/state file (`state.yaml`) | the map (`planr map show`, authoritative item state) |
| One active task | `planr pick` (single owner, heartbeat, stale recovery) |
| Task receipts | `planr log` / `planr done` (files, commands, results) |
| Goal oracle / completion proof | goal contract + live verification log |
| Scout/Judge/Worker roles | worker/reviewer subagents + `$planr-status` for honest reads |
| Final audit before done | `$planr-review` with `review close --verdict complete` |

Using such tools for intake or visualization alongside Planr is fine — keep one rule: the Planr map stays the single source of truth for item status, links, picks, reviews, approvals, and completion.

## Rules That Keep Goal Runs Honest

- Never weaken a stored goal contract mid-run; scope changes go through `$planr-plan` and the user.
- "Done" means the feature ran (live verification log), not that it compiles.
- The maker never closes its own review; single-agent hosts record `review-mode` honestly.
- Two iterations without map-state movement -> stop and report instead of grinding.
- Destructive or out-of-repo side effects always go behind `planr approval request`.
- Lessons that should outlive the iteration (a confirmed approach, a correction, a dead end) go into `planr context add "..." --tag lesson` — the next iteration or a fresh session recovers them with `planr context list --tag lesson`, not from chat history.

See also: [Skills](SKILLS.md), [Operating Model](OPERATING_MODEL.md), [Task Graph Model](TASK_GRAPH_MODEL.md), [Codex](CODEX.md), [Claude Code](CLAUDE_CODE.md), [Cursor](CURSOR.md).
