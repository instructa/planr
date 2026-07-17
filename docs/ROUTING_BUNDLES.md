# Routing bundles

Planr Core is provider-neutral. It parses `.planr/agents.toml`, resolves routes into pick packets, records declared-versus-observed evidence, and safely previews or applies a strict RoutingBundle v1:

```bash
planr routing bundle inspect routing-bundle.json
planr routing bundle preview routing-bundle.json
planr routing bundle apply routing-bundle.json
```

Core accepts only allowlisted repository-local targets, verifies payload hashes, rejects absolute paths, traversal, symlinks, parent/child target collisions, conflicts, unsupported versions, and invalid payloads, and applies the validated set atomically. It never writes user configuration or files outside the repository.

A signed bundle is accepted only with an independent trust anchor supplied to every inspect, preview, or apply call:

```bash
planr routing bundle inspect signed-bundle.json \
  --trusted-signer planr-maintainers \
  --trusted-public-key-file /absolute/path/to/maintainer.pub
```

The bundle contains the signer id and signature, not a self-trusted public key. Both trust flags are required together; unsigned bundles require neither. An unsigned bundle also cannot label its evidence `verified` or `recommended`.

The `planr-routing` workspace package owns all volatile opinions: named policies, exact model ids, host bindings, generated role and skill files, capability probes, evaluation scenarios, signing, registry data, and the website catalog. A normal flow is:

```bash
planr-routing policy list
planr-routing compile balanced --host codex-openai --output routing-bundle.json
planr routing bundle preview routing-bundle.json
planr routing bundle apply routing-bundle.json
```

The package emits the same provider-neutral bundle contract for Codex, Claude Code, Cursor, and mixed-host configurations. Offline evaluation remains experimental; a recommendation requires complete authenticated live-host evidence. Missing authentication or missing effective model, effort, role, or context-fork evidence cannot pass.
