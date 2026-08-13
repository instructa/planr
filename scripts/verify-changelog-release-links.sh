#!/usr/bin/env sh
# Verify that Keep-a-Changelog comparison references match the prepared release.
set -eu

cd "$(dirname "$0")/.."

version="${1:-}"
if ! node scripts/release-contract.mjs validate-version "$version"; then
  echo "usage: scripts/verify-changelog-release-links.sh <x.y.z[-alpha.N|-beta.N|-rc.N]>" >&2
  exit 1
fi

if ! previous_version="$(node scripts/release-contract.mjs predecessor "$version")"; then
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
