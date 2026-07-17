# Model Routing Policy

`planr-routing` owns opinionated model selection and host bindings. Planr Core owns only the provider-neutral `.planr/agents.toml` contract, route resolution, run evidence, and safe repository-local application of a RoutingBundle v1.

This split is a hard boundary:

- `planr-routing` selects models, reasoning effort, role topology, and host-native artifacts.
- Planr Core validates, previews, and transactionally applies the resulting bundle.
- Neither component writes user configuration such as `~/.codex/config.toml`.
- There is no dynamic Rust plugin, compatibility command, hidden fallback, or second policy owner in Core.

## Compile and apply

Build a deterministic bundle from one package-owned policy and host binding:

```bash
cargo run -p planr-routing -- compile balanced --host codex-openai --output /tmp/balanced-codex.json
```

Inspect and preview it with provider-neutral Planr Core before applying:

```bash
planr routing bundle inspect /tmp/balanced-codex.json
planr routing bundle preview /tmp/balanced-codex.json
planr routing bundle apply /tmp/balanced-codex.json
```

Application is repository-bound, conflict-safe, atomic, and idempotent. Planr rejects absolute paths, path traversal, symlink crossings, parent/child target collisions, home/global targets, repository `.codex/config.toml`, unsupported versions, invalid hashes or signatures, and changed existing files. Signed bundles additionally require an independently supplied trusted signer and public-key file; unsigned bundles cannot self-assert `verified` or `recommended` evidence. There is no force mode.

The Codex binding generates these repository-local owners as one bundle:

- `.planr/agents.toml` for provider-neutral profiles and routes;
- `.codex/agents/*.toml` for exact Codex model and reasoning-effort pins;
- `.codex/skills/planr-native-routing/SKILL.md` for native dispatch instructions.

Claude Code, Cursor, and mixed-host bindings generate their own repository-local artifacts from the same policy vocabulary. They never require global host configuration changes.

## Native Codex dispatch

The generated Codex role TOML is the sole model and effort owner. A dispatcher names its registered underscore-form `agent_type` and supplies a bounded context fork:

```text
spawn_agent({
  agent_type: "planr_terra_high",
  fork_turns: "none",
  message: "Implement the selected bounded Planr item."
})
```

Use `fork_turns: "none"` or an evidenced positive bounded integer whenever a selected role changes model or reasoning effort. Do not use `fork_turns: "all"`: native Codex rejects that combination rather than switching the inherited model.

Global Codex CLI `-m` and `-c model_reasoning_effort=...` flags also override spawned role files. Leave those flags unset when role-local child routing is required. Effective model, effort, native subagent source, registered role, and fork value must be verified from an authenticated Codex run; declared configuration alone is not proof.

## Registry behavior in Core

The generated `.planr/agents.toml` remains optional and provider-neutral. It contains:

- `[profiles.<id>]` entries with passthrough client, model, effort, cost tier, capabilities, and skill metadata;
- `[[routes]]` matching work types or plans;
- one `[route_default]`.

Resolution precedence is per-item override, work-type route, plan route, then default. Picks carry the resolved advisory route. Workers can attach actual-versus-declared run evidence so silent host overrides remain visible. Without a registry, Planr simply omits routing from picks.

Core inspection remains generic:

```bash
planr agents check
planr agents list
planr pick --json
```

`planr agents init` creates only a commented provider-neutral starter registry. It does not select a provider, install model policies, or render host roles.

## Policy and catalog lifecycle

Package-owned commands list policies, compile bundles, probe host capabilities, evaluate candidates, and build or verify the public catalog:

```bash
planr-routing policy list
planr-routing policy show balanced --host codex-openai
planr-routing evaluate balanced --host codex-openai
planr-routing catalog build --output website/data/catalog.json
planr-routing catalog verify website/data/catalog.json
```

Catalog candidates stay experimental and unrecommended until all required evaluation and authenticated live-host evidence is present. Missing authentication, an unavailable effective model or effort, or absent native-dispatch evidence can never be promoted to a successful verification.

## Ownership rule

Model names, named policies such as `balanced`, Sol/Terra/Luna topology, host bindings, capability probes, evaluation scenarios, registry signing, and catalog publication belong only to `planr-routing`. Planr Core may understand the generic bundle envelope and generic routing declaration, but must never regain those opinions.
