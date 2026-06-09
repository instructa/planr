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

target="${PLANR_TARGET:-$(detect_target)}"
target_dir="dist/planr-$version"
asset="planr-$target.tar.gz"

rm -rf "$target_dir" "dist/$asset"
mkdir -p "$target_dir"

cargo build --release
cp target/release/planr "$target_dir/planr"
cp README.md LICENSE.md "$target_dir/"

(
  cd "$target_dir"
  shasum -a 256 planr README.md LICENSE.md > SHA256SUMS
)

(
  cd "$target_dir"
  tar -czf "../$asset" planr README.md LICENSE.md SHA256SUMS
)

(
  cd dist
  shasum -a 256 "$asset" > SHA256SUMS
)

echo "release artifact prepared at $target_dir"
echo "checksums: $target_dir/SHA256SUMS"
echo "download asset: dist/$asset"
