# Planr Preset Catalog

This directory is a dependency-free static website. Production catalog data is never authored by hand: the repository regeneration script runs Planr's canonical evaluator and registry verifier, rewrites the report and manifest hashes, projects the verified policy/binding content, and writes `data/catalog.json`.

The committed native-v2 catalog is generated from the included `registry/manifest.toml` and canonical `registry/verification.json`. Private release and telemetry signing keys are intentionally not committed. The current Sol/Terra/Luna entry is unsigned, experimental, and unrecommended because only offline evidence exists; regeneration preserves that demotion instead of inventing or retaining a recommendation. A release operator may promote it only after a fresh trusted live oracle passes and the resulting manifest is signed offline.

```sh
cargo build --release
npm run catalog:regenerate -- --planr-bin target/release/planr --at-unix 1784160000

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

Release operators must regenerate recommendation-capable evidence through `planr agents preset evaluate` with an independently pinned telemetry collector, then use the offline maintainer signing workflow before changing the manifest status. The public `registry/report.md` summarizes the currently shipped evidence; the machine report and, when present, signature remain the verification source of truth.

The report-wide `reproducible_evidence` flag summarizes the entire candidate matrix. Registry publication independently requires the selected candidate's complete reproducible evidence, passing thresholds, verified route receipts, and matching entry in `report.recommended`; candidates that fail those gates are not published as recommendations.

Recommendation-state rendering is covered only by clearly synthetic, non-provider in-memory unit data. The runtime website has one data source, `data/catalog.json`; no file-backed recommendation fixture, alternate query path, or fabricated provider evidence is shipped.
