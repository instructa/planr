# Host Dispatch

Load this only for an explicit delegation or parallel branch, an incompatible or
lost maker, or a distinct checker for a material ReviewGate. The invoking
`$planr-loop` session is the default sequential maker. Never use this adapter to
start a coordinator and then a maker for ordinary sequential work.

- Codex: generated `.codex/config.toml` roles are native roles. Bind the exact matching `routing.profile` as `agent_type` with `fork_turns: "none"`. When no generated role matches and the default Codex role is the correct fallback, still set `fork_turns: "none"` for every Planr maker and checker spawn. Spawn each role once, wait once with `wait_agent({ timeout_ms: 3600000 })`, and never poll every 60 seconds; list/recover only after that wait times out, lost state, or explicit user steering. Do not replace role binding with prose, `task_name`, or worker identity, and do not wait on an invalid default-role spawn when a generated role is required.

  Codex maker continuity example:

  ```text
  spawn_agent({
    task_name: "maker_outcome_batch",
    agent_type: "<routing.profile>",
    fork_turns: "none",
    message: "Use $planr-work on item <item-id> as the first item in a compatible same-plan maker run. Keep one worker identity, branch only on the typed work packet, settle each ordinary outcome with planr done --next, write a compact durable handoff only at a genuine stop, and stop when settlement opens a ReviewGate, the packet is a hold, work blocks, ownership is incompatible, the pick is empty, or the budget is reached."
  })
  wait_agent({ timeout_ms: 3600000 })
  ```

  `planr done --next` atomically settles the outcome, rolls a capped internal ExecutionBatch for the same maker when required, and leases only the next compatible code outcome from the same plan. A `batch_cap_reached` transition that already contains a successor batch and next packet is not a host stop and must not wake the root or trigger `followup_task`. A response with `next.reason: "verification_handoff_source_frozen"` is a stop. The coordinator runs the packet's `commands.verify` command itself and does not route verification to the maker or another model. Maker replacement remains reserved for a genuinely unavailable, context-lost, or ownership-incompatible maker.

  If that material review returns findings, a code pick for the responsible maker returns the same ReviewGate as `work_packet.kind: "outcome", mode: "finding_repair"`. Route that packet to the same live maker agent; no fix item is created:

  ```text
  followup_task({
    target: "maker_outcome_batch",
    message: "Use $planr-work on the finding-repair work packet for ReviewGate <review-gate-id>. Keep the same PLANR_WORKER_ID, repair only the named finding ids, log changed files and commands on the gate's scoped outcome, resolve those ids on the same gate, and stop for re-review."
  })
  wait_agent({ timeout_ms: 3600000 })
  ```

  After the checker accepts the same review gate and Planr exposes `<next-compatible-item-id>`, reuse that maker again:

  ```text
  followup_task({
    target: "maker_outcome_batch",
    message: "Use $planr-work on item <next-compatible-item-id> as the next item in the same compatible same-plan maker run. Keep the same PLANR_WORKER_ID, settle ordinary outcomes with planr done --next, write a compact durable handoff only at a genuine stop, and stop at the next material review, blocker, incompatible pick, empty batch, or budget boundary."
  })
  wait_agent({ timeout_ms: 3600000 })
  ```

  The coordinator owns the verifier phase. Run `commands.verify` once under a stable worker identity that differs from the responsible maker. Do not call `spawn_agent`. The Evidence broker owns the adapter process and returns coverage and FeatureRun settlement in the same result.

  Spawn a replacement maker only when `list_agents` or the maker report shows the original is unavailable, its context is lost, ownership is incompatible, or Planr has no compatible continuation for that worker identity.
- Claude Code: plugin agents preload the worker and reviewer skills. Standalone installs provision `.claude/agents/`. The coordinator runs `commands.verify` without another agent.
- Cursor: dispatch the provisioned `.cursor/agents/planr-worker.md` and `planr-reviewer.md` roles explicitly. The coordinator runs `commands.verify` without another agent.
- Pi: invoke the repository-native router with `/skill:planr`. If optional `pi-subagents` is already installed, dispatch `.pi/agents/planr-worker.md` and `planr-reviewer.md`; Planr does not install that package. Without it, use separate sequential Pi processes for maker and checker passes and record `single_agent` honestly. Core Pi has no Planr MCP or hooks, so run `planr prime` manually when fresh context is needed. After reviewing and trusting the project resources, exact headless process dispatch has the form `pi --approve --model <provider/model> --thinking <level> -p "Use /skill:planr-work on item <item-id> as the first item in a compatible same-plan maker run. Keep one worker identity, branch only on the typed work packet, settle each ordinary outcome with planr done --next, write a compact durable handoff only at a genuine stop, and stop when settlement opens a ReviewGate, the packet is a hold, work blocks, ownership is incompatible, the pick is empty, or the budget is reached."` The coordinator runs `commands.verify` without another agent.
- Single-agent hosts: use separate sequential maker and checker passes with a fresh map read. Planr records `single_agent` honestly. The coordinator still uses `planr evidence verify`, which rejects source-mismatched receipts transactionally.

The driver should inspect routing with `planr pick --peek --plan <plan-id>` so the worker takes the actual lease. Host declarations are requests, never proof. Record observed profile/route evidence through worker completion metadata.
