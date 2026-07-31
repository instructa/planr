#!/usr/bin/env sh
set -eu

version="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n 1)"
detect_target() {
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    darwin) os="darwin" ;;
    linux) os="linux" ;;
    *)
      echo "unsupported OS: $os" >&2
      exit 1
      ;;
  esac

  case "$arch" in
    arm64 | aarch64) arch="arm64" ;;
    x86_64 | amd64) arch="x86_64" ;;
    *)
      echo "unsupported architecture: $arch" >&2
      exit 1
      ;;
  esac

  echo "$os-$arch"
}

sha256_tool() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$@"
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  else
    echo "shasum or sha256sum is required" >&2
    exit 1
  fi
}

target="${PLANR_TARGET:-$(detect_target)}"
cargo_target="${PLANR_CARGO_TARGET:-}"
dist_dir="${PLANR_DIST_DIR:-dist}"
target_dir="$dist_dir/planr-$version"
asset="planr-$target.tar.gz"

mkdir -p "$dist_dir"
rm -rf "${target_dir:?}" "${dist_dir:?}/$asset"
mkdir -p "$target_dir"

if [ -n "$cargo_target" ]; then
  cargo build --release --target "$cargo_target"
  built_bin="target/$cargo_target/release/planr"
  built_validator="target/$cargo_target/release/planr-host-capability-validator"
else
  cargo build --release
  built_bin="target/release/planr"
  built_validator="target/release/planr-host-capability-validator"
fi

cp "$built_bin" "$target_dir/planr"
mkdir -p "$target_dir/scripts"
cp "$built_validator" "$target_dir/scripts/planr-host-capability-validator"
cp scripts/host-capability-experiment.mjs "$target_dir/scripts/"
cp -R scripts/host-capability-runtime "$target_dir/scripts/"
cp README.md LICENSE.md "$target_dir/"

(
  cd "$target_dir"
  find planr scripts README.md LICENSE.md -type f -print | LC_ALL=C sort | while IFS= read -r file; do
    sha256_tool "$file"
  done > SHA256SUMS
)

(
  cd "$target_dir"
  tar -czf "../$asset" planr scripts README.md LICENSE.md SHA256SUMS
)

# Aggregate checksums over every asset present in dist/ so multi-target
# builds into the same dist directory produce one complete SHA256SUMS.
(
  cd "$dist_dir"
  sha256_tool planr-*.tar.gz > SHA256SUMS
)

echo "release artifact prepared at $target_dir"
echo "checksums: $target_dir/SHA256SUMS"
echo "download asset: $dist_dir/$asset"
