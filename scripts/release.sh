#!/usr/bin/env sh
# The only supported release path. Bumps every distribution manifest from one
# input version, runs the quality and leak gates, then commits, tags, and
# pushes. A release that skips this script fails the tag-time gate in
# .github/workflows/release.yml.
#
# Usage: scripts/release.sh <x.y.z> "one-line release summary"
#
# Preconditions:
# - branch is main with a clean worktree
# - CHANGELOG.md already contains the committed `## [x.y.z]` section
set -eu

cd "$(dirname "$0")/.."

version="${1:-}"
summary="${2:-}"
if ! echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "usage: scripts/release.sh <x.y.z> \"one-line release summary\"" >&2
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

# Gates. cargo test includes the drift guard that asserts every manifest
# carries the crate version; the leak gate mirrors CI secret scanning.
cargo build --quiet
cargo test
npm pack --dry-run
scripts/security-local.sh

git add Cargo.toml Cargo.lock package.json \
  plugins/planr/.codex-plugin/plugin.json \
  plugins/planr/.claude-plugin/plugin.json
git commit -m "release $version: $summary"
git tag "v$version"
git push origin HEAD "v$version"

echo "released v$version; watch the Release workflow for binaries and the tap update"
