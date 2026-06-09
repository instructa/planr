#!/usr/bin/env sh
set -eu

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
npm pack --dry-run
node npm/bin/planr.js --version
./.planr/tooling/test_planr

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

if [ -d /Users/kregenrek/projects/planr-test ]; then
  (
    cd /Users/kregenrek/projects/planr-test
    npm test
    npm run test:npm-planr
  )
else
  echo "planr-test not found; skipping external consumer E2E"
fi
