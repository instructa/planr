#!/usr/bin/env sh
set -eu

prefix="${PREFIX:-$HOME/.local}"
bin_dir="$prefix/bin"
repo="${PLANR_REPO:-instructa/planr}"
version="${PLANR_VERSION:-latest}"
release_base_url="${PLANR_RELEASE_BASE_URL:-}"

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

download_file() {
  url="$1"
  out="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$out"
  else
    echo "curl or wget is required to download Planr release assets" >&2
    exit 1
  fi
}

sha256_file() {
  file="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  else
    echo "shasum or sha256sum is required to verify Planr release assets" >&2
    exit 1
  fi
}

verify_archive() {
  archive="$1"
  checksum_url="$2"
  checksum_file="$3"

  if [ "${PLANR_SKIP_CHECKSUM:-0}" = "1" ]; then
    echo "checksum verification skipped by PLANR_SKIP_CHECKSUM=1"
    return
  fi

  download_file "$checksum_url" "$checksum_file"
  expected="$(awk -v asset="$asset" '$2 == asset {print $1}' "$checksum_file" | head -n 1)"
  if [ -z "$expected" ]; then
    echo "checksum for $asset not found in $checksum_url" >&2
    exit 1
  fi
  actual="$(sha256_file "$archive")"
  if [ "$actual" != "$expected" ]; then
    echo "checksum mismatch for $asset" >&2
    echo "expected: $expected" >&2
    echo "actual:   $actual" >&2
    exit 1
  fi
  echo "verified checksum for $asset"
}

install_binary() {
  source_bin="$1"

  if [ ! -f "$source_bin" ]; then
    echo "planr binary not found at $source_bin" >&2
    exit 1
  fi

  mkdir -p "$bin_dir"
  cp "$source_bin" "$bin_dir/planr"
  chmod 755 "$bin_dir/planr"
}

if [ -n "${PLANR_BIN:-}" ]; then
  install_binary "$PLANR_BIN"
elif [ "${PLANR_DOWNLOAD:-0}" != "1" ] && [ -f target/release/planr ]; then
  install_binary target/release/planr
else
  target="${PLANR_TARGET:-$(detect_target)}"
  asset="planr-$target.tar.gz"
  if [ -n "$release_base_url" ]; then
    url="${release_base_url%/}/$asset"
  elif [ "$version" = "latest" ]; then
    url="https://github.com/$repo/releases/latest/download/$asset"
  else
    url="https://github.com/$repo/releases/download/$version/$asset"
  fi

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT INT TERM
  archive="$tmp_dir/$asset"
  checksums="$tmp_dir/SHA256SUMS"

  echo "downloading $url"
  download_file "$url" "$archive"
  verify_archive "$archive" "${url%/*}/SHA256SUMS" "$checksums"
  tar -xzf "$archive" -C "$tmp_dir"

  release_bin="$(find "$tmp_dir" -type f -name planr | head -n 1)"
  if [ ! "$release_bin" ]; then
    echo "planr binary not found in release asset $asset" >&2
    exit 1
  fi

  install_binary "$release_bin"
fi

echo "installed planr to $bin_dir/planr"
echo "No global Codex, Claude Code, Cursor, or shell config was edited."
echo "Run: planr doctor --client all"
