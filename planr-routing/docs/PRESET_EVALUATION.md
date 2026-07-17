# Policy Evaluation

Package-owned evaluation checks policy composition, route coverage, capability constraints, and the canonical host contract:

```bash
planr-routing evaluate balanced --host codex-openai
```

The CLI evaluation is intentionally offline. It can prove that the selected policy and binding compose and satisfy declarative gates, but it reports no authenticated live-host evidence and can never recommend a configuration by itself.

Recommendation requires complete live evidence for every required dimension:

- the expected native host and registered role actually ran;
- effective model and reasoning effort match the binding;
- effective fork mode matches the bounded dispatch contract;
- execution completed with the expected outcome;
- evidence came from an authenticated host run rather than copied declarations.

For Codex, a valid live oracle includes the parent and child transcript identities, native `subagent` source, exact `agent_role`, explicit `fork_turns`, effective child model and effort, and a task-specific completion marker. Missing authentication or any unavailable effective field is a failed oracle, never a successful or inferred verification.

The Sol/Terra/Luna suite also proves that native cross-role dispatch rejects `fork_turns = "all"`, uses registered underscore-form agent types, has no fallback chain, and keeps role TOMLs as the sole model/effort owners. Global Codex CLI model or effort flags must be absent from this oracle because they override child role files.

Catalog generation remains fail-closed: all generated entries are experimental and `recommended = false` until authenticated live evidence and the independent signing gate exist.
