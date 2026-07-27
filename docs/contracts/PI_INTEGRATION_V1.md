# Pi Integration Contract v1

Status: frozen for implementation

Frozen: 2026-07-27

Upstream source probes:

- `earendil-works/pi` `main` commit
  `a597371bda2af70372d1323d550483b5f4a0ae36` (2026-07-27).
- Pi release `v0.82.1` commit
  `b4f293684bba718d59cc1157679bcf6157b3a7f5` (2026-07-27).
- `nicobailon/pi-subagents` `main` commit `73fb2fc` (2026-07-27).

The Pi repository currently publishes release tags ahead of the fetched
`main` version line, so this contract records both: `main` is the requested
source probe, while `v0.82.1` is the latest released runtime/API baseline.

This contract is the implementation boundary for first-class Pi support.
It follows Pi's native extension model instead of treating every coding agent
as an MCP host. Pi core deliberately ships without MCP, subagents, plan mode,
or permission popups. It natively discovers Agent Skills and can run their CLI
workflows through its built-in `bash` tool.

## Frozen artifact and discovery policy

`planr install pi` owns exactly the repository paths in the executable
contract below. Workflow files are copied byte-for-byte from the canonical
`plugins/planr/skills` sources. Pi-specific role bodies exist only because
`pi-subagents` frontmatter is not interchangeable with Claude/Cursor role
frontmatter: it uses `thinking`, explicit tool/skill inheritance controls, and
project agents under `.pi/agents/**/*.md`.

Pi loads `.pi/skills/` only after the repository is trusted. Interactive Pi
asks for trust; non-interactive `-p`, JSON, and RPC runs ignore untrusted
project resources by default. A headless operator who has deliberately chosen
to trust the repository passes `--approve` for that run. Planr never writes
Pi's user trust store or `.pi/settings.json`.

`pi-subagents` is optional. When installed, it discovers both Planr roles.
Without it, the role files are inert and the same Planr skills remain usable
in the parent Pi session or separate Pi processes.

```json
{
  "contract_version": 1,
  "pi_main_revision": "a597371bda2af70372d1323d550483b5f4a0ae36",
  "pi_release": {
    "tag": "v0.82.1",
    "revision": "b4f293684bba718d59cc1157679bcf6157b3a7f5"
  },
  "pi_subagents_revision": "73fb2fc",
  "artifacts": [
    {"target": ".pi/agents/planr-reviewer.md", "source": "plugins/planr/agents/pi/planr-reviewer.md", "kind": "agent"},
    {"target": ".pi/agents/planr-worker.md", "source": "plugins/planr/agents/pi/planr-worker.md", "kind": "agent"},
    {"target": ".pi/skills/planr-goal/SKILL.md", "source": "plugins/planr/skills/planr-goal/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-loop/SKILL.md", "source": "plugins/planr/skills/planr-loop/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-loop/agents/planr-reviewer.md", "source": "plugins/planr/skills/planr-loop/agents/planr-reviewer.md", "kind": "skill_asset"},
    {"target": ".pi/skills/planr-loop/agents/planr-worker.md", "source": "plugins/planr/skills/planr-loop/agents/planr-worker.md", "kind": "skill_asset"},
    {"target": ".pi/skills/planr-loop/references/host-dispatch.md", "source": "plugins/planr/skills/planr-loop/references/host-dispatch.md", "kind": "skill_asset"},
    {"target": ".pi/skills/planr-loop/references/recovery-and-verification.md", "source": "plugins/planr/skills/planr-loop/references/recovery-and-verification.md", "kind": "skill_asset"},
    {"target": ".pi/skills/planr-plan/SKILL.md", "source": "plugins/planr/skills/planr-plan/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-review/SKILL.md", "source": "plugins/planr/skills/planr-review/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-status/SKILL.md", "source": "plugins/planr/skills/planr-status/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-summary/SKILL.md", "source": "plugins/planr/skills/planr-summary/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-task-graph/SKILL.md", "source": "plugins/planr/skills/planr-task-graph/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-verify-web/SKILL.md", "source": "plugins/planr/skills/planr-verify-web/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr-work/SKILL.md", "source": "plugins/planr/skills/planr-work/SKILL.md", "kind": "skill"},
    {"target": ".pi/skills/planr/SKILL.md", "source": "plugins/planr/skills/planr/SKILL.md", "kind": "skill"}
  ],
  "mcp": {"supported": false, "artifacts": []},
  "extensions": {"emitted": false, "artifacts": []},
  "hooks": {"supported": false, "artifacts": []},
  "settings": {"emitted": false, "artifacts": []},
  "trust": {
    "project_resources_require_trust": true,
    "interactive": "Pi prompts the operator",
    "headless": "pass --approve only after choosing to trust the repository"
  },
  "observed_client": {
    "environment": "PI_CODING_AGENT",
    "accepted_value": "true",
    "stored_value": "pi",
    "advisory_only": true
  },
  "client_all_includes_pi": false,
  "headless": {
    "command": "pi --approve --model <provider/model> --thinking <level> -p \"<prompt>\""
  }
}
```

## Pi role contract

Both role files use `pi-subagents` project-agent frontmatter:

- `name` is exactly `planr-worker` or `planr-reviewer`;
- `systemPromptMode: replace`;
- `inheritProjectContext: true`;
- `inheritSkills: false`;
- `skills` names exactly `planr-work` or `planr-review`;
- the worker tool allowlist includes read/search, `bash`, `edit`, and `write`;
- the reviewer tool allowlist includes read/search and mutation-capable `bash`,
  but omits the `edit` and `write` tools;
- the reviewer declares `acceptanceRole: read-only` and
  `completionGuard: false`, so pi-subagents treats the bash-enabled validator
  as non-implementation work without changing its tool access;
- no role pins `model`, `thinking`, `fallbackModels`, provider credentials, or
  extensions.

Model and thinking selection remain external host policy. Operators can use
Pi or `pi-subagents` settings/overrides, while Planr records only requested and
observed evidence.

## Merge and overwrite policy

- Missing target: write the canonical packaged source.
- Existing identical content: unchanged.
- Existing different content: preserve without `--force`; replace only that
  target with `--force`.
- `--no-mcp` and `--no-hooks` are accepted for install-command parity but
  change no Pi path because the v1 artifact set contains neither.
- Dry-run enumerates the same paths the corresponding write reconciles.
- `project init --client pi` provisions the same skills and roles without
  writing any other Pi artifact.
- `--client all` remains the legacy Codex, Claude Code, and Cursor expansion.

## Runtime observation boundary

Pi documents that commands launched by its built-in tools receive
`PI_CODING_AGENT=true`. Planr accepts only that exact value as advisory
`runs.observed_client=pi`. Other values and other ambient `PI_*` variables are
ignored. The marker is not authentication, authorization, proof of model,
proof of `pi-subagents`, or worker identity.

Pi has no Planr hook in v1. Run `planr prime` manually after startup or
compaction when a fresh state block is useful. Agent Skills remain the primary
entry point: `/skill:planr` explicitly loads the router, while natural-language
requests may trigger Pi's skill discovery.

## Security and CI boundary

Planr installation must not read or write:

- `~/.pi`, Pi auth files, provider keys, OAuth tokens, sessions, transcripts,
  prompts, responses, model catalogs, package caches, or trust decisions;
- `.pi/settings.json`, `.pi/extensions/`, `.pi/prompts/`, or an MCP config;
- global `pi install`, `pi config`, package enablement, or user settings.

Release and CI workflows do not install/invoke Pi, `pi-subagents`, a provider,
or a live model. Deterministic tests parse the frozen contract, compile/write
assets into disposable repositories, and validate CLI/docs output.

## Sources

- `packages/coding-agent/README.md` at Pi `v0.82.1`: project trust, skills,
  extensions, packages, philosophy, CLI, and `PI_CODING_AGENT`.
- `packages/coding-agent/docs/skills.md` at Pi `v0.82.1`.
- `nicobailon/pi-subagents` README at `73fb2fc`: project agent discovery,
  prompt assembly, frontmatter, and model overrides.
- `nicobailon/pi-subagents` `test/unit/agent-frontmatter.test.ts` and
  `test/unit/path-resolution.test.ts` at `73fb2fc`.
- https://github.com/earendil-works/pi
- https://github.com/nicobailon/pi-subagents
