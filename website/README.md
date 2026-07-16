# Planr Preset Catalog

This directory is a dependency-free static website. Production catalog data is never authored by hand: `build-catalog.mjs` invokes Planr's canonical registry verifier, requires a trusted maintainer signature and safe policy/binding preview, binds status to the evaluation report, and writes `data/catalog.json`.

The committed catalog is generated from the included `registry/manifest.toml`, canonical `registry/verification.json`, and separately pinned public maintainer trust store. Its private release and telemetry signing keys are intentionally not committed. If verification expires, is deprecated, or no longer earns recommendation, the generated catalog preserves the visible lifecycle state instead of inventing or retaining a recommendation.

```sh
node website/build-catalog.mjs \
  --planr-bin ./target/release/planr \
  --manifest website/registry/manifest.toml \
  --content-root . \
  --trust-store website/registry/trusted-maintainers.toml \
  --entry balanced-codex=codex \
  --at-unix 1783987200 \
  --output website/data/catalog.json

npm run site:test
npm run site:serve
```

## Cloudflare test deployment

Cloudflare infrastructure is owned by the repository-root `alchemy.run.ts`. The
deployment publishes a clean `dist/website` containing only runtime assets; tests,
fixtures, registry inputs, trust stores, and build tooling are never uploaded.

Install the pinned development dependency and authenticate Cloudflare using an existing
Alchemy/Cloudflare profile, Wrangler login, or a least-privilege API token. Set the
account id from `.env.example` in the process environment when account inference would
be ambiguous. Planr never runs login/configure commands and never changes user-level
configuration. This static stack has no secret bindings, so it deliberately requires no
`ALCHEMY_PASSWORD` or repository-local secret file.

The published Planr CLI retains its declared Node.js 18+ compatibility. Repository
dependency installation and the separate Cloudflare deployment toolchain require Node.js
22+ because the pinned pnpm/Alchemy lockfile contains current tooling with that minimum.
Select Node 22 or newer before invoking pnpm. On an unknown machine, this pnpm-free
preflight is safe to run first even with Node 18 or 20:

```sh
node scripts/check-alchemy-runtime.mjs
```

The authoritative deployment launcher performs the same check before it starts pnpm or
Alchemy, so older runtimes receive Planr's error instead of a Corepack or dependency crash.

```sh
pnpm install
pnpm site:check
node scripts/cloudflare-test.mjs deploy
```

After Node 22+ is active, `pnpm deploy:test` is an equivalent convenience alias.

The committed `pnpm-workspace.yaml` is a repository-only dependency security policy:
it allows the `esbuild` and `workerd` install scripts required by Alchemy and explicitly
keeps the unused transitive `sharp` build disabled. It does not modify global pnpm state.

`deploy:test` always targets the isolated Alchemy stage `test`. It creates a standalone
Cloudflare Worker website and prints its Cloudflare-assigned `workers.dev` URL. The
initial deployment deliberately configures no custom domain, DNS route, backend resource,
or adoption of an existing Worker.

Removing that test website is an explicit, separate operation:

```sh
node scripts/cloudflare-test.mjs destroy
```

After Node 22+ is active, `pnpm destroy:test` is an equivalent convenience alias.

Never add private registry or telemetry signing keys to the deployment environment. The
site deploy consumes only the already verified public `website/data/catalog.json`.

Release operators must regenerate the evaluation report through `planr agents preset evaluate` with an independently pinned telemetry collector, update the manifest artifact digest, and sign the registry entry with the corresponding offline maintainer key before rebuilding this file. The public `registry/report.md` summarizes that evidence; the machine report and signature remain the verification source of truth.

The report-wide `reproducible_evidence` flag summarizes the entire candidate matrix. Registry publication independently requires the selected candidate's complete reproducible evidence, passing thresholds, verified route receipts, and matching entry in `report.recommended`; candidates that fail those gates are not published as recommendations.

The localhost-only `?fixture=recommended` query remains available for isolated UI regression checks. It is visibly labeled, ignored on non-local hosts, and never substitutes for live verification of the production `data/catalog.json` path.
