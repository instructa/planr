#!/usr/bin/env sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
planr_test_dir=${PLANR_TEST_DIR:-"$(dirname "$repo_root")/planr-test"}

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm run verify:release-eval-gate
PLANR_ORACLE_SELF_TEST=source-provenance node scripts/verify-switchloom-cross-product.mjs
cargo build --release
npm pack --dry-run
node npm/bin/planr.js --version

if command -v shellcheck >/dev/null 2>&1; then
  shellcheck scripts/*.sh
else
  echo "shellcheck not found; install shellcheck to run the shell lint gate" >&2
  exit 1
fi

if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit --deny warnings
else
  echo "cargo-audit not found; install cargo-audit to run the dependency audit gate" >&2
  exit 1
fi

if [ -d "$planr_test_dir" ]; then
  (
    cd "$planr_test_dir"
    npm test
    npm run test:npm-planr
  )
else
  echo "planr-test not found at $planr_test_dir; skipping external consumer E2E"
fi
