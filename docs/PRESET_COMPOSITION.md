# Preset Composition

Planr composes two independent inputs:

- a provider-neutral Usage Policy v1 TOML file;
- a versioned host binding containing exact client/model/effort and dispatch capabilities.

Preview is the default:

```bash
planr agents preset apply ./policy.toml --binding ./cursor-binding.toml --preview --json
planr agents preset apply ./policy.toml --binding ./cursor-binding.toml --confirm --json
```

The equivalent MCP tool is `planr_preset_apply` with `policy`, `binding`, optional `confirm`, and—when any profile targets Codex—the same `live_host_command`, `live_host_args`, `trusted_telemetry_signer`, and `trusted_telemetry_collector` inputs used by live evaluation. Planr reads the local version from `codex --version`; native-v2 and every requested model/effort must be proven by challenge-bound receipts from the hash-pinned collector. Caller assertions are not accepted.

## Built-in Catalog and Safe Packs

`planr agents preset list --json` (MCP: `planr_presets_list`) lists the four embedded policy presets, five host bindings, source checksums, and 20 declared safe pairs. Built-ins resolve by id or id plus `.toml`, so they work from any repository without copying package files first:

```bash
planr agents preset apply balanced --binding codex-openai --live-host-command /absolute/codex-adapter --trusted-telemetry-signer codex --trusted-telemetry-collector /absolute/collector --preview --json
planr agents preset apply read-only-audit --binding cursor-openai --confirm --json
```

Policies: `balanced`, `low-usage`, `max-quality`, and `read-only-audit`. Bindings: `codex-openai`, `cursor-openai`, `cursor-fable-grok`, `claude-native`, and `mixed-host`. The exact inspectable sources ship under `presets/policies/` and `presets/bindings/` and are also compiled into the binary.

A response whose inputs came from the embedded catalog carries `pack.status = "safe"` and `pack.safe = true`. Explicit file inputs remain custom even when their internal ids match a built-in; they are allowed, carry `pack.status = "custom"`, and add a warning requiring review of compatibility, permission, and artifact diffs. This prevents copied or modified files from inheriting safe-pack status by name alone.

## Host Binding v1

Bindings use strict TOML: unknown fields fail parsing. Abstract role keys must cover every role in `policy.execution.roles`; bindings may add a driver profile without adding it to execution permissions.

```toml
schema_version = 1
id = "codex-native"
version = "2.0.0"
host = "codex"
driver_role = "driver"
default_role = "driver"
capability_evidence = ["codex-0.144.0-native-v2-9ff47868eb"]
billing_assumptions = ["local subscription"]
known_limitations = ["native multi-agent v2 and the requested model must be active"]

[capabilities]
model_override = true
effort_override = true
fork_none = true
fork_all = true

[profiles.driver]
profile = "codex-sol-medium"
client = "codex"
model = "gpt-5.6-sol"
effort = "medium"

[profiles.explorer]
profile = "codex-terra-medium"
client = "codex"
model = "gpt-5.6-terra"
effort = "medium"
fork_turns = { mode = "none" }

[profiles.worker]
profile = "codex-terra-high"
client = "codex"
model = "gpt-5.6-terra"
effort = "high"
skill = "planr-work"
fork_turns = { mode = "none" }

[profiles.mechanical]
profile = "codex-luna-xhigh"
client = "codex"
model = "gpt-5.6-luna"
effort = "xhigh"
skill = "planr-work"
fork_turns = { mode = "none" }

[profiles.reviewer]
profile = "codex-sol-high"
client = "codex"
model = "gpt-5.6-sol"
effort = "high"
skill = "planr-review"
fork_turns = { mode = "none" }

[profiles.moonshot]
profile = "codex-sol-ultra"
client = "codex"
model = "gpt-5.6-sol"
effort = "ultra"
fork_turns = { mode = "none" }

[[routes]]
work_type = "exploration"
role = "explorer"

[[routes]]
work_type = "code"
role = "worker"

[[routes]]
work_type = "mechanical"
role = "mechanical"

[[routes]]
work_type = "review"
role = "reviewer"

[[routes]]
work_type = "moonshot"
role = "moonshot"

[verification]
id = "verify-codex-native"
verified_at_unix = 1900000000
max_age_seconds = 2592000

[[artifacts]]
path = ".codex/agents/planr-terra-high.toml"
kind = "codex_agent"
content = '''name = "planr_terra_high"
description = "Normal implementation and testing."
model = "gpt-5.6-terra"
model_reasoning_effort = "high"
developer_instructions = "Use planr-work for one picked item."
'''

[[artifacts]]
path = ".codex/skills/planr-native-routing/SKILL.md"
kind = "codex_skill"
content = '''---
name: planr-native-routing
description: Dispatch the repository-owned native Codex roles.
---
Dispatch code with `spawn_agent({ agent_type: "planr-terra-high", fork_turns: "none", ... })`. The role TOML alone owns model and effort. Native Codex rejects `fork_turns: "all"` when `agent_type` selects a role with model or effort overrides.
'''
```

Native Codex rejects `fork_turns = { mode = "all" }` when `agent_type` selects a role with model or effort overrides. Omitted fork configuration defaults to `none`. A partial fork accepts any positive integer and requires non-empty `capability_evidence`; zero is invalid. Secret-like values are forbidden in all binding metadata, including capability evidence, billing assumptions, limitations, profile metadata, routes, and artifact paths/kinds; composition rejects them with field-only diagnostics before producing dispatch/warning output or mutating the repository.

## Preview and Lock

The preview includes:

- composed abstract-role to registry-profile and dispatch-context mappings;
- compatibility errors and host/billing limitations;
- execution-permission additions relative to the active policy;
- verification id, age, maximum age, and fresh/stale state;
- source ids, versions, SHA-256 hashes, Planr version, and applied-at value;
- every target's kind, size, proposed/existing hash, `create`, `unchanged`, or `conflict` action, and deterministic old/proposed configuration. TOML is projected structurally so policy limits/transitions and registry routes/profiles/fallbacks are directly auditable; other text is line-projected. Secret-like keys and credential-shaped values are replaced with `[REDACTED]`.

Confirmed apply writes the previewed bytes and records `policy_applied`. The lock at `.planr/preset.lock.toml` contains source hashes and hashes for every generated non-lock artifact. Tests and reproducible packaging can pin `PLANR_PRESET_NOW_UNIX` and `PLANR_PRESET_APPLIED_AT`; ordinary operation records current time.

## Repository Boundary

Only these repository-relative targets are valid:

- `.planr/policy.toml`
- `.planr/agents.toml`
- `.planr/preset.lock.toml`
- files below `.codex/{agents,skills}/`, `.claude/{agents,skills}/`, or `.cursor/{agents,skills}/`

Planr rejects absolute paths, `.`/`..` traversal, non-normalized paths, symlink crossings, home/global targets, and repository `.codex/config.toml` before the first write. Existing different files are conflicts and are never overwritten; there is no force or privileged mode.
