# Signed Catalog Registry

`planr-routing` owns catalog generation and detached Ed25519 signatures. Planr Core has no named-policy registry and no network dependency.

Build and verify the deterministic public catalog from package-owned sources:

```bash
planr-routing catalog build --output website/data/catalog.json
planr-routing catalog verify website/data/catalog.json
```

All entries remain experimental and unrecommended unless the evaluation gate has authenticated live-host evidence. A signature authenticates bytes; it cannot promote incomplete evidence.

Create and verify a detached signature with explicitly supplied local files:

```bash
planr-routing registry sign website/data/catalog.json \
  --signer planr-maintainers \
  --private-key-file /absolute/path/to/private-key \
  --output /tmp/catalog.signature.json

planr-routing registry verify website/data/catalog.json \
  --signature /tmp/catalog.signature.json \
  --trusted-signer planr-maintainers \
  --trusted-public-key-file /absolute/path/to/maintainer.pub
```

Signing never searches user configuration, keychains, environment variables, or home-directory defaults. The caller chooses the input, private-key file, signer id, and output. The detached signature does not carry a trusted public key; verification requires the expected signer and an independently provisioned key file. It fails closed for signer mismatch, malformed signatures, wrong content, or invalid key material.

The website publishes generated catalog data only. Runtime route resolution and previously applied repository files do not depend on the website, network availability, or registry state.
