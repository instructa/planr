# Preset Registry

Planr's registry is an optional distribution boundary, never a runtime dependency. Active projects keep using their repository-local `.planr/policy.toml`, `.planr/agents.toml`, and preset lock when the registry is unavailable. Previously imported packs remain available under `.planr/registry/cache/` for deterministic offline use.

## Trust and integrity

A schema-v1 registry manifest identifies a registry version and one or more versioned entries. Each entry declares:

- kind (`policy`, `host-binding`, or `pack`), lifecycle (`published`, `deprecated`, or `revoked`), and evaluation status;
- Planr version bounds and compatible hosts;
- verification and review timestamps, a verification artifact, and optional replacement/revocation metadata;
- for `verified` or `recommended` entries, the exact policy, binding, and evaluation-suite ids and versions covered by that evidence;
- the normalized path, declarative kind, byte length, and SHA-256 digest of every artifact;
- an optional Ed25519 signature.

Signatures do not establish their own trust. Planr verifies them only against keys provisioned separately in `.planr/registry/trusted-maintainers.toml` (or an explicitly selected trust store):

```toml
schema_version = 1

[[maintainers]]
id = "planr-maintainers"
public_key = "<64 hexadecimal Ed25519 public-key characters>"
revoked = false
```

An unsigned pack can be checksum-verified, but a manifest's `recommended` claim is demoted to `verified` unless a non-revoked pinned maintainer signature verifies. A trusted signature is necessary but not sufficient: Planr also re-runs canonical policy/binding composition and safe-artifact validation, binds the shipped bytes to the current built-in evaluation inputs, and validates the current suite/task provenance. Candidate metrics and threshold gates are derived again from the task results; result hashes, task-oracle coverage, the canonical Codex dispatch contract, trusted route/metering evidence, and the report recommendation record must all agree before `recommended` survives. A merely signed status label or stale suite therefore cannot promote an entry.

Invalid signatures, revoked entries/signers, incompatible Planr/host constraints, checksum/size mismatches, traversal, symlinks, binary or executable content, secret-like content, semantically invalid bindings, and malformed declarative artifacts fail closed. Every registry policy, including an `experimental` entry without evaluation metadata, uses the same public-distribution safety check: commands, hooks, environment grants, network/MCP access, secret references, and overwrite permission are rejected. Manifest ids, versions, hosts, references, reasons, paths, and signer metadata are screened field-by-field for secret-like values before they can reach diagnostics; public keys and signature bytes are shape-validated but deliberately not classified as secrets. Stale verification or deprecation remains visible and removes recommendation.

## Verification and offline import

Remote retrieval is deliberately outside the command. Downloading a manifest and its content is an explicit operator action; Planr then verifies local inputs:

```sh
planr agents preset registry verify registry.toml \
  --entry balanced-codex \
  --content-root ./download \
  --host codex
```

`--host` may be omitted only for entries whose `compatible_hosts` list is empty. A constrained entry with no explicit host fails closed.

Import is preview-first. The preview lists exactly the manifest-declared files and deterministic cache target. `--confirm` copies only those verified files into a manifest-hash-addressed immutable directory and atomically stores both the original registry manifest and a cache receipt:

```sh
planr agents preset registry import registry.toml \
  --entry balanced-codex \
  --content-root ./download \
  --host codex

planr agents preset registry import registry.toml \
  --entry balanced-codex \
  --content-root ./download \
  --host codex \
  --confirm
```

`planr agents preset registry list` requires no source manifest or network. The mutable receipt is inventory metadata, not a checksum or trust authority. Every read binds the cache path to the stored manifest hash, re-verifies the original manifest and maintainer signature against the repository trust store, compares receipt inventory with the manifest, and recomputes artifact sizes and hashes from the manifest. Coordinated edits to content and receipt therefore remain detectable. The command reports `current` or `stale` freshness from the review timestamp and marks malformed or tampered cache entries unusable. Extra source files are never imported.

The equivalent MCP tools are `planr_preset_registry_verify`, `planr_preset_registry_import`, and `planr_preset_registry_list`. MCP import also previews unless `confirm` is true.
