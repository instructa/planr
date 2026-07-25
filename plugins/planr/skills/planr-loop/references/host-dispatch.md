# Host Dispatch

Load this only when the loop has subagents and must bind a provider-neutral Planr profile to host-native dispatch.

- Codex: generated `.codex/config.toml` roles are native roles. Bind the exact matching `routing.profile` as `agent_type` with `fork_turns: "none"`. Do not replace binding with prose, `task_name`, or worker identity, and do not wait on an invalid default-role spawn when a generated role is required.
- Claude Code: plugin agents preload the worker/reviewer skills; standalone installs provision `.claude/agents/`.
- Cursor: dispatch the provisioned `.cursor/agents/planr-worker.md` and `planr-reviewer.md` roles explicitly.
- Single-agent hosts: use separate sequential maker/checker passes with a fresh map read; Planr records `single_agent` honestly.

The driver should inspect routing with `planr pick --peek --plan <plan-id>` so the worker takes the actual lease. Host declarations are requests, never proof. Record observed profile/route evidence through worker completion metadata.
