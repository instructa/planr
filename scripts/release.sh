#!/usr/bin/env sh
# Publish an already prepared, reviewed release commit. This script never
# changes, stages, or commits source files; it only gates and tags exact main.
set -eu

cd "$(dirname "$0")/.."

version="${1:-}"
summary="${2:-}"
if ! node scripts/release-contract.mjs validate-version "$version"; then
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
scripts/verify-changelog-release-links.sh "$version"
node scripts/release-contract.mjs verify-predecessor "$version"
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

ci_receipt="${PLANR_RELEASE_CI_RECEIPT:-}"
approval="${PLANR_RELEASE_APPROVAL:-}"
if [ -z "$ci_receipt" ] || [ -z "$approval" ]; then
  echo "PLANR_RELEASE_CI_RECEIPT and PLANR_RELEASE_APPROVAL are required" >&2
  exit 1
fi

# Promote independent exact-SHA evidence. Evaluation evidence is required by
# the verifier only when the evaluated subject or its explicit policy changed.
set -- \
  --version "$version" \
  --ci-receipt "$ci_receipt" \
  --approval "$approval"
if [ -n "${PLANR_RELEASE_EVAL_RECEIPT:-}" ]; then set -- "$@" --eval-receipt "$PLANR_RELEASE_EVAL_RECEIPT"; fi
if [ -n "${PLANR_RELEASE_EVAL_SUITE:-}" ]; then set -- "$@" --eval-suite "$PLANR_RELEASE_EVAL_SUITE"; fi
if [ -n "${PLANR_RELEASE_EVAL_DB:-}" ]; then set -- "$@" --eval-db "$PLANR_RELEASE_EVAL_DB"; fi
if [ -n "${PLANR_RELEASE_PLANR_BIN:-}" ]; then set -- "$@" --planr-bin "$PLANR_RELEASE_PLANR_BIN"; fi
node scripts/verify-release-promotion.mjs "$@"

git tag -a "v$version" -m "planr v$version: $summary"
git push origin HEAD "v$version"

echo "released reviewed candidate v$version; watch the Release workflow"
