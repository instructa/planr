#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."

# Official multi-architecture Rust image. The manifest-list digest fixes both
# native amd64 and arm64 toolchains to Rust 1.90.0 on musl/Alpine 3.21.
build_image="rust:1.90.0-alpine3.21@sha256:3757b14ddcc2057eb91a074dcdd0913bed839b22444bd2229a49eea910ed8736"
expected_rust="1.90.0"
target="${PLANR_TARGET:-}"
cargo_target="${PLANR_CARGO_TARGET:-}"

if [ "$(uname -s)" != "Linux" ]; then
  echo "Linux release artifacts must be built on a native Linux CI runner" >&2
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

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for the pinned Linux release toolchain" >&2
  exit 1
fi

cache_parent="${RUNNER_TEMP:-/tmp}"
cargo_home="$(mktemp -d "$cache_parent/planr-linux-cargo.XXXXXX")"
trap 'rm -rf "$cargo_home"' EXIT HUP INT TERM

docker run --rm \
  --platform "$docker_platform" \
  --user "$(id -u):$(id -g)" \
  --env CARGO_HOME=/cargo-home \
  --env RUSTUP_HOME=/usr/local/rustup \
  --env PLANR_TARGET="$target" \
  --env PLANR_CARGO_TARGET="$cargo_target" \
  --env PLANR_EXPECTED_RUST="$expected_rust" \
  --volume "$PWD:/work" \
  --volume "$cargo_home:/cargo-home" \
  --workdir /work \
  "$build_image" \
  sh -eu -c '
    case "$(rustc --version)" in
      "rustc $PLANR_EXPECTED_RUST "*) ;;
      *) echo "unexpected pinned Rust toolchain: $(rustc --version)" >&2; exit 1 ;;
    esac
    scripts/build-release.sh
  '

echo "portable_linux_build=passed target=$target cargo_target=$cargo_target rust=$expected_rust image=${build_image#*@}"
