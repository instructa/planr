# Policy Composition

`planr-routing` composes one package-owned usage policy with one package-owned host binding into a deterministic RoutingBundle v1. The package is the only owner of named policies, provider model ids, reasoning levels, host-specific role topology, and generated host instructions.

```bash
planr-routing policy list
planr-routing policy show balanced --host codex-openai
planr-routing compile balanced --host codex-openai --output /tmp/balanced-codex.json
```

The supported policy/host matrix is explicit. Unknown policy ids, host ids, unsupported pairs, unknown TOML fields, incomplete abstract-role mappings, unsafe artifact paths, secret-like metadata, and invalid fork policies fail before a bundle is emitted.

## Composition contract

A policy declares abstract roles, routes, capabilities, and usage constraints. A binding maps every required abstract role to an exact host profile and may add a driver profile. Composition produces:

- a provider-neutral `.planr/agents.toml`;
- host-local role files and dispatch skills;
- content hashes for every artifact;
- source and composition provenance inside the versioned bundle.

Codex role changes use `fork_turns = { mode = "none" }` or a positive bounded partial fork with explicit capability evidence. `all` is rejected when the role changes model or effort. Generated dispatch examples use the registered underscore-form agent type, such as `planr_terra_high`; file names may remain hyphenated.

Global Codex CLI `-m` and `-c model_reasoning_effort=...` flags have higher precedence than spawned role files. Generated skills therefore tell drivers to leave those flags unset when role-local child routing is required.

## Safe application boundary

The package never applies files itself. Planr Core consumes the generic bundle:

```bash
planr routing bundle inspect /tmp/balanced-codex.json
planr routing bundle preview /tmp/balanced-codex.json
planr routing bundle apply /tmp/balanced-codex.json
```

Core permits only allowlisted repository-local targets under `.planr/`, `.codex/agents/`, `.codex/skills/`, `.claude/agents/`, `.claude/skills/`, `.cursor/agents/`, and `.cursor/skills/`. It rejects absolute or home paths, traversal, symlink crossings, parent/child target collisions, `.codex/config.toml`, conflicts, malformed hashes or signatures, and unsupported bundle versions. Signed bundles require an external trusted signer and public-key file. Apply is atomic and idempotent; there is no overwrite or privileged mode.

This keeps volatile provider policy independently releasable while retaining the dangerous filesystem transaction in the trusted provider-neutral Core.
