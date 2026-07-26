#!/usr/bin/env sh
set -eu

sh scripts/check-repository-privacy.sh
node scripts/hooks/block-forbidden-staged-files.mjs

if command -v betterleaks >/dev/null 2>&1; then
  if [ -f .betterleaks.toml ]; then
    betterleaks git --no-banner --redact=100 --config .betterleaks.toml .
  else
    betterleaks git --no-banner --redact=100 .
  fi
else
  echo "betterleaks not found; install betterleaks to run the secret leak gate" >&2
  exit 1
fi

if command -v trivy >/dev/null 2>&1; then
  trivy fs \
    --scanners vuln,secret,misconfig \
    --ignorefile .trivyignore.yaml \
    --skip-dirs target \
    --skip-dirs dist \
    --skip-dirs node_modules \
    --exit-code 1 \
    .
else
  echo "trivy not found; install trivy to run the filesystem security gate" >&2
  exit 1
fi
