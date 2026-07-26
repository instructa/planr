#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."

# Official multi-architecture Rust image. The manifest-list digest fixes both
# native amd64 and arm64 toolchains to Rust 1.90.0 on musl/Alpine 3.21.
build_image="rust:1.90.0-alpine3.21@sha256:3757b14ddcc2057eb91a074dcdd0913bed839b22444bd2229a49eea910ed8736"
expected_rust="1.90.0"
musl_version="1.2.5-r11"
apk_base="https://dl-cdn.alpinelinux.org/alpine/v3.21/main"
target="${PLANR_TARGET:-}"
cargo_target="${PLANR_CARGO_TARGET:-}"

if [ "$(uname -s)" != "Linux" ]; then
  echo "Linux release artifacts must be built on a native Linux CI runner" >&2
  exit 1
fi

case "$(uname -m):$target:$cargo_target" in
  x86_64:linux-x86_64:x86_64-unknown-linux-musl)
    docker_platform="linux/amd64"
    apk_arch="x86_64"
    musl_sha256="61e84757a8bfbc0d7fa8f4ce6de9cd4d791714369d78f6a08e5b03510fb2a623"
    musl_dev_sha256="d3b5ab01046a92b9a168b790f516606e320f015cbd4deeb584c5e115a02124ba"
    ;;
  aarch64:linux-arm64:aarch64-unknown-linux-musl | arm64:linux-arm64:aarch64-unknown-linux-musl)
    docker_platform="linux/arm64"
    apk_arch="aarch64"
    musl_sha256="721010e6bff908878d9c527428598661be59dde0d9f013f8431d01fd4dd16652"
    musl_dev_sha256="9c4ebdc7e2a29f12de5135cee8f1b92439bfff7c74839b4fb7b422680cf18db4"
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
apk_dir="$(mktemp -d "$cache_parent/planr-linux-apk.XXXXXX")"
trap 'rm -rf "$cargo_home" "$apk_dir"' EXIT HUP INT TERM

for package in musl musl-dev; do
  curl --fail --show-error --silent --location --proto '=https' --tlsv1.2 \
    "$apk_base/$apk_arch/$package-$musl_version.apk" \
    --output "$apk_dir/$package.apk"
done
printf '%s  %s\n%s  %s\n' \
  "$musl_sha256" "$apk_dir/musl.apk" \
  "$musl_dev_sha256" "$apk_dir/musl-dev.apk" \
  | sha256sum -c -

builder_image="planr-linux-release-builder:$expected_rust-$target-$musl_version"
docker build \
  --platform "$docker_platform" \
  --file scripts/linux-release-builder.Dockerfile \
  --build-arg "BUILD_IMAGE=$build_image" \
  --build-arg "MUSL_VERSION=$musl_version" \
  --build-arg "MUSL_SHA256=$musl_sha256" \
  --build-arg "MUSL_DEV_SHA256=$musl_dev_sha256" \
  --tag "$builder_image" \
  "$apk_dir"

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
  "$builder_image" \
  sh -eu -c '
    case "$(rustc --version)" in
      "rustc $PLANR_EXPECTED_RUST "*) ;;
      *) echo "unexpected pinned Rust toolchain: $(rustc --version)" >&2; exit 1 ;;
    esac
    scripts/build-release.sh
  '

echo "portable_linux_build=passed target=$target cargo_target=$cargo_target rust=$expected_rust musl=$musl_version image=${build_image#*@}"
