#!/usr/bin/env sh
# Prepare the exact source state that will later be reviewed and published.
# This script never stages, commits, tags, pushes, or publishes anything.
set -eu

cd "$(dirname "$0")/.."

version="${1:-}"
if ! echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$'; then
  echo "usage: scripts/prepare-release-candidate.sh <x.y.z[-alpha.N|-beta.N|-rc.N]>" >&2
  exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
  echo "worktree is dirty; prepare the candidate from reviewed, committed source" >&2
  exit 1
fi
if git rev-parse "v$version" >/dev/null 2>&1; then
  echo "tag v$version already exists" >&2
  exit 1
fi

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

pnpm install --frozen-lockfile
if ! git diff --quiet -- pnpm-lock.yaml; then
  echo "pnpm-lock.yaml changed during frozen workspace synchronization" >&2
  exit 1
fi

cargo build --quiet
pnpm --filter @planr/docs reference:generate
pnpm --filter @planr/docs reference:check

echo "prepared v$version candidate source; review and commit the resulting tracked files"
