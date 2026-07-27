#!/usr/bin/env sh
# Verify that Keep-a-Changelog comparison references match the prepared release.
set -eu

cd "$(dirname "$0")/.."

version="${1:-}"
if ! echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta|rc)\.[0-9]+)?$'; then
  echo "usage: scripts/verify-changelog-release-links.sh <x.y.z[-alpha.N|-beta.N|-rc.N]>" >&2
  exit 1
fi

if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no '## [$version]' release section" >&2
  exit 1
fi

previous_version="$(awk -v version="$version" '
  /^## \[[^]]+\]/ {
    heading = $0
    sub(/^## \[/, "", heading)
    sub(/\].*$/, "", heading)
    if (found) {
      print heading
      exit
    }
    if (heading == version) found = 1
  }
' CHANGELOG.md)"
if [ -z "$previous_version" ]; then
  echo "CHANGELOG.md has no release section before $version" >&2
  exit 1
fi

unreleased="[Unreleased]: https://github.com/instructa/planr/compare/v$version...HEAD"
release="[$version]: https://github.com/instructa/planr/compare/v$previous_version...v$version"
if ! grep -Fqx "$unreleased" CHANGELOG.md; then
  echo "CHANGELOG.md [Unreleased] comparison must start at v$version" >&2
  exit 1
fi
if ! grep -Fqx "$release" CHANGELOG.md; then
  echo "CHANGELOG.md [$version] comparison must span v$previous_version...v$version" >&2
  exit 1
fi

echo "changelog_release_links=passed version=$version previous=$previous_version"
