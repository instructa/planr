# Planr Preset Catalog

This directory is a dependency-free static website owned by `planr-routing`. Production catalog data is never authored by hand: the package compiler reads the canonical usage policies, host bindings, and evaluation suite, then writes all 20 policy/binding compositions to `data/catalog.json`.

Private signing keys are intentionally never committed. Generated entries remain unsigned, experimental, and unrecommended while only offline evidence exists. A release operator may promote an entry only after a fresh authenticated live-host oracle proves every required effective-routing dimension and the catalog is signed offline.

```sh
cargo build --release --manifest-path Cargo.toml
pnpm catalog:regenerate -- --routing-bin ../target/release/planr-routing

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

Release operators must first capture authenticated effective model, effort, native-subagent, and bounded-fork evidence. `planr-routing evaluate` fails closed to `experimental` when any dimension is absent. Offline signatures are produced with `planr-routing registry sign`; private key files stay outside the repository.

The checked-in catalog is reproducible with `planr-routing catalog build` and verified byte-for-byte with `planr-routing catalog verify`.

Recommendation-state rendering is covered only by clearly synthetic, non-provider in-memory unit data. The runtime website has one data source, `data/catalog.json`; no file-backed recommendation fixture, alternate query path, or fabricated provider evidence is shipped.
