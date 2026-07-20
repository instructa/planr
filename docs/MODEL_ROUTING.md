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

Host-specific model policies, generated repository roles, and uninstall/application lifecycle are external to Planr. [Switchloom](https://switchloom.ai) [v0.2.1](https://github.com/instructa/switchloom/releases/tag/v0.2.1) can manage repository-local routing artifacts from its [repository](https://github.com/instructa/switchloom), [setup quickstart](https://github.com/instructa/switchloom/blob/v0.2.1/README.md#setup-from-the-website), and [repository lifecycle docs](https://github.com/instructa/switchloom/blob/v0.2.1/docs/preset-composition.md#repository-lifecycle-commands), but Planr does not install, invoke, compile, apply, or uninstall Switchloom output. After an external tool or operator changes `.planr/agents.toml` or `.planr/policy.toml`, run `planr agents check` and capture observed route evidence through `planr log add` or `planr done`.

The hard boundary is provider-neutral: Planr consumes declarations and evidence; hosts and external routing tools decide whether any requested model, effort, role, or fallback actually ran. See [External Routing Declarations](ROUTING_BUNDLES.md).
