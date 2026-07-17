# Model routing

Planr Core treats routing as optional, advisory repository data. `.planr/agents.toml` declares opaque profiles and routes; Planr resolves them into pick packets but never calls a provider or claims that a requested model actually ran.

```toml
[profiles.worker]
client = "host-a"
model = "model-id"
agent_type = "repository-role"
effort = "high"
skill = "planr-work"

[[routes]]
match = { work_type = "code" }
profile = "worker"
```

Create a neutral scaffold or an explicit registry:

```bash
planr agents init
planr agents init --profile worker=host-a/model-id@high#standard --route code=worker
planr agents check
planr agents list --json
```

Resolution order is per-item override, work type, plan, then default route. Unknown profiles fail open to the next applicable route. Host names, model ids, role selectors, effort values, and fallback behavior are opaque to Core.

Workers may report observed routing with logs and route-audit evidence. Requested-only values never become effective proof; missing effective evidence remains explicitly unavailable.

Host-specific model policies and generated repository roles are optional. The `planr-routing` workspace package compiles them into RoutingBundle v1, and Core safely previews and applies that bundle:

```bash
planr-routing compile balanced --host codex-openai --output routing-bundle.json
planr routing bundle preview routing-bundle.json
planr routing bundle apply routing-bundle.json
```

Bundle application is restricted to the repository. Planr never edits user configuration such as `~/.codex/config.toml`. See [Routing Bundles](ROUTING_BUNDLES.md).
