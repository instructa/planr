#!/usr/bin/env sh
# Publish an already prepared, reviewed release commit. This script never
# changes, stages, or commits source files; it only gates and tags exact main.
set -eu

cd "$(dirname "$0")/.."

version="${1:-}"
summary="${2:-}"
if ! echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$'; then
  echo "usage: scripts/release.sh <x.y.z[-alpha.N|-beta.N|-rc.N]> \"one-line release summary\"" >&2
  exit 1
fi
if [ -z "$summary" ]; then
  echo "usage: scripts/release.sh <x.y.z> \"one-line release summary\"" >&2
  exit 1
fi

branch="$(git rev-parse --abbrev-ref HEAD)"
if [ "$branch" != "main" ]; then
  echo "release must run on main (current: $branch)" >&2
  exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "worktree is dirty; publish only the reviewed candidate commit" >&2
  exit 1
fi
if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no committed '## [$version]' section" >&2
  exit 1
fi
if git rev-parse "v$version" >/dev/null 2>&1; then
  echo "tag v$version already exists" >&2
  exit 1
fi

crate_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
if [ "$crate_version" != "$version" ]; then
  echo "Cargo.toml is $crate_version, not prepared candidate $version" >&2
  exit 1
fi
lock_version="$(sed -n '/name = "planr"/{n;s/^version = "\([^"]*\)"/\1/p;q;}' Cargo.lock)"
if [ "$lock_version" != "$version" ]; then
  echo "Cargo.lock is $lock_version, not prepared candidate $version" >&2
  exit 1
fi
for manifest in \
  package.json \
  plugins/planr/.codex-plugin/plugin.json \
  plugins/planr/.claude-plugin/plugin.json \
  .cursor-plugin/plugin.json; do
  manifest_version="$(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$manifest" | head -n 1)"
  if [ "$manifest_version" != "$version" ]; then
    echo "$manifest is $manifest_version, not prepared candidate $version" >&2
    exit 1
  fi
done

# Recheck generated, version-derived references and frozen workspace state.
# Every command must leave the reviewed commit byte-clean.
pnpm install --frozen-lockfile
pnpm --filter @planr/docs reference:check
cargo build --quiet
if [ -n "$(git status --porcelain)" ]; then
  echo "release verification changed the reviewed candidate source" >&2
  exit 1
fi

eval_receipt="${PLANR_RELEASE_EVAL_RECEIPT:-}"
eval_suite="${PLANR_RELEASE_EVAL_SUITE:-}"
eval_db="${PLANR_RELEASE_EVAL_DB:-}"
if [ -z "$eval_receipt" ] || [ -z "$eval_suite" ] || [ -z "$eval_db" ]; then
  echo "PLANR_RELEASE_EVAL_RECEIPT, PLANR_RELEASE_EVAL_SUITE, and PLANR_RELEASE_EVAL_DB are required" >&2
  exit 1
fi
node scripts/verify-release-eval-receipt.mjs \
  --receipt "$eval_receipt" \
  --db "$eval_db" \
  --suite "$eval_suite" \
  --planr-bin target/debug/planr

cargo test
npm run verify:release-eval-gate
npm pack --dry-run
scripts/security-local.sh
if [ -n "$(git status --porcelain)" ]; then
  echo "release gates changed the reviewed candidate source" >&2
  exit 1
fi

git tag -a "v$version" -m "planr v$version: $summary"
git push origin HEAD "v$version"

echo "released reviewed candidate v$version; watch the Release workflow"
