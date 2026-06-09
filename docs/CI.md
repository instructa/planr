# CI

Planr CI is defined in `.github/workflows/ci.yml` and `.github/workflows/security.yml`.

## Required Gates

The main CI workflow runs:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
shellcheck scripts/*.sh
cargo build --release
npm pack --dry-run
node npm/bin/planr.js --version
./.planr/tooling/test_planr
cargo audit --deny warnings
```

The security workflow runs a GitHub Actions security scan with pinned `zizmor`.

## Local Reproduction

Run the full local gate:

```bash
scripts/ci-local.sh
```

`scripts/ci-local.sh` also runs the external consumer E2E project when `/Users/kregenrek/projects/planr-test` exists:

```bash
cd /Users/kregenrek/projects/planr-test
npm test
npm run test:npm-planr
```

Run local security and leak checks:

```bash
scripts/security-local.sh
```

This uses BetterLeaks for secret history scanning and Trivy for filesystem vulnerability, secret, and misconfiguration scanning.

## Supply Chain

Dependabot is configured in `.github/dependabot.yml` for:

- Cargo dependencies
- npm dependencies
- GitHub Actions

GitHub Actions use read-only default permissions and pin checkout by commit SHA.
