#!/usr/bin/env sh
set -eu

if command -v betterleaks >/dev/null 2>&1; then
  betterleaks git --no-banner --redact=100 .
else
  echo "betterleaks not found; install betterleaks to run the secret leak gate" >&2
  exit 1
fi

if command -v trivy >/dev/null 2>&1; then
  trivy fs \
    --scanners vuln,secret,misconfig \
    --skip-dirs target \
    --skip-dirs dist \
    --skip-dirs node_modules \
    --exit-code 1 \
    .
else
  echo "trivy not found; install trivy to run the filesystem security gate" >&2
  exit 1
fi
