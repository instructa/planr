#!/bin/sh
# Planr Codex Stop hook. It fails open: hook infrastructure problems must
# never prevent Codex from stopping.
set -u
command -v planr >/dev/null 2>&1 || exit 0
tmp=$(mktemp "${TMPDIR:-/tmp}/planr-codex-stop.XXXXXX") || exit 0
trap 'rm -f "$tmp"' EXIT HUP INT TERM
cat >"$tmp" || exit 0
planr --json stop --input "$tmp" 2>/dev/null || true
exit 0
