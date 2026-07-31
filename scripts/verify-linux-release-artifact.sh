#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."

# Alpine 3.20 predates the build image and is pinned by its multi-architecture
# manifest-list digest. A static artifact must complete the lifecycle here
# without inheriting the Ubuntu build runner's glibc.
runtime_image="alpine:3.20.8@sha256:765942a4039992336de8dd5db680586e1a206607dd06170ff0a37267a9e01958"
target="${PLANR_TARGET:-}"
cargo_target="${PLANR_CARGO_TARGET:-}"
version="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n 1)"

if [ "$(uname -s)" != "Linux" ]; then
  echo "Linux release verification must run on a native Linux CI runner" >&2
  exit 1
fi

case "$(uname -m):$target:$cargo_target" in
  x86_64:linux-x86_64:x86_64-unknown-linux-musl) docker_platform="linux/amd64" ;;
  aarch64:linux-arm64:aarch64-unknown-linux-musl | arm64:linux-arm64:aarch64-unknown-linux-musl)
    docker_platform="linux/arm64"
    ;;
  *)
    echo "native runner, release target, and musl target do not match: $(uname -m):$target:$cargo_target" >&2
    exit 1
    ;;
esac

asset="dist/planr-$target.tar.gz"
if [ ! -f "$asset" ]; then
  echo "missing release asset: $asset" >&2
  exit 1
fi

verify_parent="${RUNNER_TEMP:-/tmp}"
verify_root="$(mktemp -d "$verify_parent/planr-linux-verify.XXXXXX")"
trap 'rm -rf "$verify_root"' EXIT HUP INT TERM
extract="$verify_root/extract"
mkdir -p "$extract"

actual_members="$(tar -tzf "$asset" | LC_ALL=C sort)"
for required in \
  LICENSE.md \
  README.md \
  SHA256SUMS \
  planr \
  scripts/host-capability-experiment.mjs \
  scripts/planr-host-capability-validator \
  scripts/host-capability-runtime/v1/schemas/host-capability-observed-raw.schema.json \
  scripts/host-capability-runtime/v1/schemas/host-capability-expected-manifest.schema.json \
  scripts/host-capability-runtime/v1/schemas/host-capability-provenance.schema.json
do
  if ! printf '%s\n' "$actual_members" | grep -Fx "$required" >/dev/null; then
    echo "release tarball is missing required path: $required" >&2
    exit 1
  fi
done
unexpected_members="$(printf '%s\n' "$actual_members" | grep -Ev '^(LICENSE.md|README.md|SHA256SUMS|planr|scripts/host-capability-experiment\.mjs|scripts/planr-host-capability-validator|scripts/host-capability-runtime/v1/schemas/host-capability-(observed-raw|expected-manifest|provenance)\.schema\.json)' || true)"
if [ -n "$unexpected_members" ]; then
  echo "release tarball contains unexpected paths:" >&2
  printf '%s\n' "$unexpected_members" >&2
  exit 1
fi
tar -xzf "$asset" -C "$extract"
(
  cd "$extract"
  sha256sum -c SHA256SUMS
)

binary="$extract/planr"
validator="$extract/scripts/planr-host-capability-validator"
test -x "$binary"
test -x "$validator"
for executable in "$binary" "$validator"; do
  file "$executable" | grep -Eq 'ELF 64-bit.*(x86-64|ARM aarch64)'
  if readelf -l "$executable" | grep -q 'INTERP'; then
    echo "Linux release binary has a dynamic program interpreter: $executable" >&2
    exit 1
  fi
  if readelf -d "$executable" | grep -q '(NEEDED)'; then
    echo "Linux release binary has dynamic shared-library dependencies: $executable" >&2
    exit 1
  fi
  if strings "$executable" | grep -Eq 'GLIBC_[0-9]'; then
    echo "Linux release binary retains a glibc symbol requirement: $executable" >&2
    exit 1
  fi
done

reported="$($binary --version)"
test "$reported" = "planr $version"
validator_identity="$($validator --identity)"
printf '%s\n' "$validator_identity" | grep -q '"validator":"planr-host-capability-validator"'

docker run --rm \
  --platform "$docker_platform" \
  --network none \
  --read-only \
  --tmpfs /tmp:rw,nosuid,nodev,mode=1777,size=64m \
  --user 65532:65532 \
  --volume "$extract:/artifact:ro" \
  --volume "$PWD/scripts/verify-public-lifecycle.sh:/verify-public-lifecycle.sh:ro" \
  "$runtime_image" \
  /bin/sh /verify-public-lifecycle.sh /artifact/planr "$version"

npm_fixture="$verify_root/npm-fixture"
mkdir -p "$npm_fixture/npm/bin" "$npm_fixture/npm/native/$target"
cp package.json "$npm_fixture/package.json"
cp npm/bin/planr.js "$npm_fixture/npm/bin/planr.js"
cp "$binary" "$npm_fixture/npm/native/$target/planr"
cp "$validator" "$npm_fixture/npm/native/$target/planr-host-capability-validator"
chmod 755 "$npm_fixture/npm/native/$target/planr"
chmod 755 "$npm_fixture/npm/native/$target/planr-host-capability-validator"
cmp "$binary" "$npm_fixture/npm/native/$target/planr"
cmp "$validator" "$npm_fixture/npm/native/$target/planr-host-capability-validator"
npm_reported="$(cd "$npm_fixture" && node npm/bin/planr.js --version)"
test "$npm_reported" = "planr $version"

echo "linux_release_verification=passed target=$target linkage=static-musl runtime=alpine-3.20.8 lifecycle=passed npm_bytes=identical validator_bytes=identical"
