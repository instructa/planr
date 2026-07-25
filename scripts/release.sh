#!/usr/bin/env sh
# The only supported release path. Bumps every distribution manifest from one
# input version, runs the quality and leak gates, then commits, tags, and
# pushes. A release that skips this script fails the tag-time gate in
# .github/workflows/release.yml.
#
# Usage: scripts/release.sh <x.y.z[-alpha.N|-beta.N|-rc.N]> "one-line release summary"
#
# Pre-release versions (e.g. 1.2.0-alpha.1) ship a GitHub prerelease and
# publish npm under the `alpha` dist-tag instead of `latest`; the
# Homebrew tap only moves on stable versions.
#
# Preconditions:
# - branch is main with a clean worktree
# - CHANGELOG.md already contains the committed `## [<version>]` section
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
  echo "worktree is dirty; commit or stash before releasing" >&2
  exit 1
fi
if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no '## [$version]' section; write and commit it first" >&2
  exit 1
fi
if git rev-parse "v$version" >/dev/null 2>&1; then
  echo "tag v$version already exists" >&2
  exit 1
fi

# One version source feeds every manifest. sed writes through a temp file so
# the script behaves identically on BSD and GNU.
replace() {
  file="$1"
  pattern="$2"
  sed "$pattern" "$file" > "$file.release-tmp"
  mv "$file.release-tmp" "$file"
}
replace Cargo.toml "s/^version = \".*\"/version = \"$version\"/"
replace package.json "s/\"version\": \".*\"/\"version\": \"$version\"/"
replace plugins/planr/.codex-plugin/plugin.json "s/\"version\": \".*\"/\"version\": \"$version\"/"
replace plugins/planr/.claude-plugin/plugin.json "s/\"version\": \".*\"/\"version\": \"$version\"/"
replace .cursor-plugin/plugin.json "s/\"version\": \".*\"/\"version\": \"$version\"/"

# The candidate binary owns eval semantics. The local receipt contains only
# identifiers/digests; model-backed evidence remains in the local Planr DB.
cargo build --quiet
eval_receipt="${PLANR_RELEASE_EVAL_RECEIPT:-}"
if [ -z "$eval_receipt" ]; then
  echo "PLANR_RELEASE_EVAL_RECEIPT is required; capture fresh local eval evidence before release" >&2
  exit 1
fi
eval_suite="${PLANR_RELEASE_EVAL_SUITE:-}"
if [ -z "$eval_suite" ]; then
  echo "PLANR_RELEASE_EVAL_SUITE is required; point it at the externally maintained suite" >&2
  exit 1
fi
eval_db="${PLANR_RELEASE_EVAL_DB:-}"
if [ -z "$eval_db" ]; then
  echo "PLANR_RELEASE_EVAL_DB is required; point it at the external eval result database" >&2
  exit 1
fi
node scripts/verify-release-eval-receipt.mjs \
  --receipt "$eval_receipt" \
  --db "$eval_db" \
  --suite "$eval_suite" \
  --planr-bin target/debug/planr

# Deterministic gates. cargo test includes the manifest drift guard; the leak
# gate mirrors CI secret scanning. All remain before commit, tag, and push.
cargo test
npm run verify:release-eval-gate
npm pack --dry-run
scripts/security-local.sh

git add Cargo.toml Cargo.lock package.json \
  plugins/planr/.codex-plugin/plugin.json \
  plugins/planr/.claude-plugin/plugin.json \
  .cursor-plugin/plugin.json
git commit -m "release $version: $summary"
git tag -a "v$version" -m "planr v$version: $summary"
git push origin HEAD "v$version"

echo "released v$version; watch the Release workflow for binaries and the tap update"
