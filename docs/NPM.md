# npm Package

The npm package name is `planr`. Published versions bundle platform-native binaries under `npm/native/<os>-<arch>/planr`, so installing from npm requires no Rust toolchain:

```bash
npm install -g planr
planr --version
```

Supported platforms: `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, `linux-arm64`. There is no postinstall script and no network download at install time; the binaries ship inside the tarball and are checksum-verified against the GitHub Release `SHA256SUMS` before publish.

## Publishing

Publishing happens only from the `npm-publish` job in `.github/workflows/release.yml` via npm Trusted Publishing (OIDC, no long-lived token). The job runs when the repository variable `NPM_PUBLISH_ENABLED` is `true` and requires a one-time Trusted Publisher configuration on npmjs.com: package `planr` -> Settings -> Publishing access -> GitHub Actions publisher with repository `instructa/planr` and workflow `release.yml`.

## Binary Resolution

The wrapper looks for a native binary in this order:

1. `PLANR_NATIVE_BIN`;
2. `npm/native/<os>-<arch>/planr` (published package);
3. `target/release/planr` then `target/debug/planr` (repository checkout).

## Local Development

The repository checkout contains no `npm/native/` binaries; the wrapper falls back to local cargo builds:

```bash
cargo build --release
npm link
planr --version
```

For consumer E2E testing:

```bash
cd ~/projects/planr-test
npm link ../planr
npm run test:npm-planr
```
