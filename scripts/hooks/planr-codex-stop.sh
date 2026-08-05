#!/bin/sh
# Planr Codex Stop hook. The hook owns no workflow policy: it forwards the
# host envelope to `planr stop`, whose plan audit reads canonical FeatureRun /
# ReviewGate execution state. Infrastructure failures still fail open.
set -u
command -v planr >/dev/null 2>&1 || exit 0
tmp=$(mktemp "${TMPDIR:-/tmp}/planr-codex-stop.XXXXXX") || exit 0
trap 'rm -f "$tmp"' EXIT HUP INT TERM
cat >"$tmp" || exit 0
planr --json stop --input "$tmp" 2>/dev/null || true
exit 0
