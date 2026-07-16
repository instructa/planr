# Preset Composition

Planr composes two independent inputs:

- a provider-neutral Usage Policy v1 TOML file;
- a versioned host binding containing exact client/model/effort and dispatch capabilities.

Preview is the default:

```bash
planr agents preset apply ./policy.toml --binding ./codex-binding.toml --preview --json
planr agents preset apply ./policy.toml --binding ./codex-binding.toml --confirm --json
```

The equivalent MCP tool is `planr_preset_apply` with `policy`, `binding`, and optional `confirm` fields. Both surfaces call one application service and return the same compatibility, permission, verification-age, provenance, conflict, and artifact shapes.

## Built-in Catalog and Safe Packs

`planr agents preset list --json` (MCP: `planr_presets_list`) lists the four embedded policy presets, five host bindings, source checksums, and 20 declared safe pairs. Built-ins resolve by id or id plus `.toml`, so they work from any repository without copying package files first:

```bash
planr agents preset apply balanced --binding codex-openai --preview --json
planr agents preset apply read-only-audit --binding mixed-host --confirm --json
```

Policies: `balanced`, `low-usage`, `max-quality`, and `read-only-audit`. Bindings: `codex-openai`, `cursor-openai`, `cursor-fable-grok`, `claude-native`, and `mixed-host`. The exact inspectable sources ship under `presets/policies/` and `presets/bindings/` and are also compiled into the binary.

A response whose inputs came from the embedded catalog carries `pack.status = "safe"` and `pack.safe = true`. Explicit file inputs remain custom even when their internal ids match a built-in; they are allowed, carry `pack.status = "custom"`, and add a warning requiring review of compatibility, permission, and artifact diffs. This prevents copied or modified files from inheriting safe-pack status by name alone.

## Host Binding v1

Bindings use strict TOML: unknown fields fail parsing. Abstract role keys must cover every role in `policy.execution.roles`; bindings may add a driver profile without adding it to execution permissions.

```toml
schema_version = 1
id = "codex-team"
version = "1.0.0"
host = "codex"
driver_role = "driver"
default_role = "driver"
capability_evidence = ["codex-0.138-cross-tier-smoke"]
billing_assumptions = ["local subscription"]
known_limitations = ["effective model requires host evidence"]

[capabilities]
model_override = true
effort_override = true
fork_none = true
fork_all = true
max_partial_fork_turns = 4

[profiles.driver]
profile = "sol"
client = "codex"
model = "gpt-5.5"
effort = "xhigh"

[profiles.worker]
profile = "luna"
client = "codex"
model = "gpt-5.4-mini"
effort = "high"
skill = "planr-work"
# Omit for the deterministic `none` default.
fork_turns = { mode = "partial", turns = 2 }

[[routes]]
work_type = "code"
role = "worker"
fallback_roles = ["driver"]

[verification]
id = "verify-codex-team"
verified_at_unix = 1900000000
max_age_seconds = 2592000

[[artifacts]]
path = ".codex/agents/luna.toml"
kind = "codex_agent"
content = '''model = "gpt-5.4-mini"
model_reasoning_effort = "high"
'''
```

Codex cross-tier children never accept `fork_turns = { mode = "all" }`, because full history inheritance defeats the requested model/effort override. Omitted fork configuration defaults to `none`. A partial fork must be positive, at or below `max_partial_fork_turns`, and backed by non-empty `capability_evidence`. Secret-like values are forbidden in all binding metadata, including capability evidence, billing assumptions, limitations, profile metadata, routes, and artifact paths/kinds; composition rejects them with field-only diagnostics before producing dispatch/warning output or mutating the repository.

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
