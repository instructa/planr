#!/bin/bash
# planr evidence guard (advisory): a subagent that stops while holding a
# pick without a completion log gets one follow-up reminder. Never
# blocks; any failure exits silently.
set -u
command -v planr >/dev/null 2>&1 || exit 0
command -v jq >/dev/null 2>&1 || exit 0
worker="${PLANR_WORKER_ID:-${PLANR_SESSION_ID:-}}"
[ -z "$worker" ] && exit 0
held=$(planr --json map show 2>/dev/null | jq -r --arg w "$worker" '
  [.items[]? | select((.status == "picked" or .status == "running") and .worker_id == $w)] | .[0].id // empty
' 2>/dev/null)
[ -z "$held" ] && exit 0
logs=$(planr --json log list --item "$held" 2>/dev/null | jq -r '
  [.logs[]? | select(.kind == "completion")] | length
' 2>/dev/null)
if [ "${logs:-0}" = "0" ]; then
  jq -nc --arg held "$held" '{followup_message: ("planr: " + $held + " is still picked with no completion log. Log evidence (planr done " + $held + " --summary ... --cmd ...) or release it (planr pick release " + $held + ").")}'
fi
exit 0
