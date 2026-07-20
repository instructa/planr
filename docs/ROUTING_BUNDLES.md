# External routing declarations

Planr Core is provider-neutral. It parses `.planr/agents.toml`, resolves routes into pick packets, records declared-versus-observed evidence, and checks `.planr/policy.toml`. It does not own a bundle format, catalog, compiler, signer, installer, or uninstaller.

The current boundary is repository-local declaration plus evidence:

```bash
planr agents init
planr agents check
planr agents list --json
```

External tools, including Switchloom v0.2.1, may create, update, apply, or uninstall repository-local routing artifacts. Those lifecycle operations happen outside Planr. Planr only consumes the resulting provider-neutral declarations and the route observations workers attach to logs.

Because declarations are advisory, requested model values never become proof by themselves. Strong evidence keeps requested, host-resolved, and effective execution separate in route observations, with `unavailable` used when the host cannot prove what ran.
