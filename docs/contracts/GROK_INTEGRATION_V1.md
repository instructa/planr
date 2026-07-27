# Grok Build Integration Contract v1

Status: frozen for implementation

Frozen: 2026-07-27

Upstream source probe: `xai-org/grok-build` commit
`b41c75a578f98bddbd326ab02cd53618451d97ee` (2026-07-26)

This contract is the implementation boundary for first-class Grok Build support.
It reconciles the public xAI documentation, the locally cloned upstream source,
and Planr's local-first integration rules. Live authenticated verification is a
separate maintainer-local release gate; it never runs in CI.

## Frozen artifact and discovery policy

`planr install grok` owns exactly the repository paths in the executable
contract below. Workflow files are copied byte-for-byte from the canonical
`plugins/planr` sources. There is no second Grok-specific workflow body.

Planr does **not** emit `.grok/plugins/planr` or a Grok plugin manifest in v1.
Current Grok source discovers project plugins from `.grok/plugins`, but project
plugins default to disabled and project `.grok/config.toml` does not merge
`[plugins].enabled`. A repository-local plugin would therefore require
user-level enablement and would not satisfy one-command project setup. Native
`.grok/skills/<name>/` and `.grok/agents/*.md` are documented, directly
discovered project paths and do not depend on Claude compatibility scanners.

Grok's current manifest lookup is still relevant for troubleshooting: a plugin
root checks `plugin.json`, then `.grok-plugin/plugin.json`, then
`.claude-plugin/plugin.json`, and finally convention-based component folders.
That behavior is observed upstream but deliberately not used by Planr v1.

```json
{
  "contract_version": 1,
  "upstream_revision": "b41c75a578f98bddbd326ab02cd53618451d97ee",
  "artifacts": [
    {"target": ".grok/config.toml", "source": "generated:grok_project_config", "kind": "mcp"},
    {"target": ".grok/agents/planr-reviewer.md", "source": "plugins/planr/agents/planr-reviewer.md", "kind": "agent"},
    {"target": ".grok/agents/planr-worker.md", "source": "plugins/planr/agents/planr-worker.md", "kind": "agent"},
    {"target": ".grok/skills/planr-goal/SKILL.md", "source": "plugins/planr/skills/planr-goal/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-loop/SKILL.md", "source": "plugins/planr/skills/planr-loop/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-loop/agents/planr-reviewer.md", "source": "plugins/planr/skills/planr-loop/agents/planr-reviewer.md", "kind": "skill_asset"},
    {"target": ".grok/skills/planr-loop/agents/planr-worker.md", "source": "plugins/planr/skills/planr-loop/agents/planr-worker.md", "kind": "skill_asset"},
    {"target": ".grok/skills/planr-loop/references/host-dispatch.md", "source": "plugins/planr/skills/planr-loop/references/host-dispatch.md", "kind": "skill_asset"},
    {"target": ".grok/skills/planr-loop/references/recovery-and-verification.md", "source": "plugins/planr/skills/planr-loop/references/recovery-and-verification.md", "kind": "skill_asset"},
    {"target": ".grok/skills/planr-plan/SKILL.md", "source": "plugins/planr/skills/planr-plan/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-review/SKILL.md", "source": "plugins/planr/skills/planr-review/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-status/SKILL.md", "source": "plugins/planr/skills/planr-status/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-summary/SKILL.md", "source": "plugins/planr/skills/planr-summary/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-task-graph/SKILL.md", "source": "plugins/planr/skills/planr-task-graph/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-verify-web/SKILL.md", "source": "plugins/planr/skills/planr-verify-web/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr-work/SKILL.md", "source": "plugins/planr/skills/planr-work/SKILL.md", "kind": "skill"},
    {"target": ".grok/skills/planr/SKILL.md", "source": "plugins/planr/skills/planr/SKILL.md", "kind": "skill"}
  ],
  "mcp": {
    "table": "mcp_servers.planr",
    "command": "planr",
    "args": ["mcp"],
    "enabled": true,
    "startup_timeout_sec": 30,
    "tool_timeout_sec": 6000,
    "env": {"PLANR_MCP_CLIENT": "grok"},
    "forbidden_fields": ["XAI_API_KEY", "api_key", "auth", "model", "headers", "url"]
  },
  "plugin": {
    "emitted": false,
    "project_root": ".grok/plugins",
    "manifest_precedence": ["plugin.json", ".grok-plugin/plugin.json", ".claude-plugin/plugin.json", "convention"],
    "project_default": "disabled",
    "project_config_enabled_merged": false
  },
  "hooks": {"supported": false, "artifacts": []},
  "client_all_includes_grok": false,
  "headless": {"command": "grok --no-auto-update -p \"<prompt>\" --output-format json"}
}
```

The generated `.grok/config.toml` table is therefore:

```toml
[mcp_servers.planr]
command = "planr"
args = ["mcp"]
enabled = true
startup_timeout_sec = 30
tool_timeout_sec = 6000
env = { PLANR_MCP_CLIENT = "grok" }
```

`PLANR_MCP_CLIENT=grok` is a narrow adapter marker applied by Grok's MCP
launcher to the Planr child process. Planr may use the exact value as an
advisory observed-client signal. It is not authentication, authorization, or a
general worker identity. Ambient `GROK_*` variables are not accepted as the v1
signal because the probed Grok MCP launcher does not inject a stable one.

## Merge and overwrite policy

- Missing `.grok/config.toml`: create it with only the Planr MCP table.
- Existing valid config: perform a document-preserving TOML merge. Preserve
  every foreign table, key, comment, and order; only `mcp_servers.planr` is
  owned by Planr.
- Existing different `mcp_servers.planr`: without `--force`, preserve it and
  report a conflict; with `--force`, replace only that table.
- Malformed config: fail with a path-and-parse diagnostic and write nothing,
  including under `--force`.
- Workflow target missing: write the canonical source. Existing identical
  content is unchanged. Existing different content is preserved without
  `--force` and replaced from the canonical source with `--force`.
- `--no-mcp` omits `.grok/config.toml` reconciliation. `--no-hooks` is accepted
  for CLI parity but changes no Grok paths because v1 installs no hooks.
- Dry-run enumerates the same paths the corresponding write would reconcile.

## Security and CI boundary

Repository artifacts contain no xAI credential, token, auth-file path, model
selection, endpoint, or absolute maintainer path. Planr never reads or copies
`~/.grok`, `$GROK_HOME`, session data, `auth.json`, or
`mcp_credentials.json` while installing.

GitHub Actions and release workflows must not install or invoke Grok, reference
`XAI_API_KEY`, gate on an xAI secret, or perform live inference. Deterministic
tests parse and compare generated files without Grok installed. Maintainer-local
release verification may use an existing login, but recorded evidence is
limited to redacted discovery/origin, diagnostics, and the disposable Planr
project identity.

The supported inspection and headless syntax is `grok inspect --json` and
`grok --no-auto-update -p "<prompt>" --output-format json`; `grok exec` is not part of this contract.

## Sources

- https://docs.x.ai/build/features/skills-plugins-marketplaces
- https://docs.x.ai/build/settings
- https://docs.x.ai/build/features/mcp-servers
- https://docs.x.ai/build/cli/reference
- https://docs.x.ai/build/cli/headless-scripting
- https://github.com/xai-org/grok-build/tree/b41c75a578f98bddbd326ab02cd53618451d97ee
