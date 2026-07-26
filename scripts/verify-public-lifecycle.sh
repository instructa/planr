#!/usr/bin/env sh
set -eu

planr_bin="${1:-}"
expected_version="${2:-}"
if [ -z "$planr_bin" ] || [ -z "$expected_version" ] || [ ! -x "$planr_bin" ]; then
  echo "usage: scripts/verify-public-lifecycle.sh /absolute/path/to/planr <version>" >&2
  exit 1
fi

workspace="$(mktemp -d /tmp/planr-public-lifecycle.XXXXXX)"
trap 'rm -rf "$workspace"' EXIT HUP INT TERM
mkdir -p "$workspace/home"
export HOME="$workspace/home"
export PLANR_WORKER_ID="public-linux-lifecycle"
cd "$workspace"

reported="$("$planr_bin" --version)"
if [ "$reported" != "planr $expected_version" ]; then
  echo "public binary reports '$reported', expected 'planr $expected_version'" >&2
  exit 1
fi

first_id() {
  sed -n 's/^[[:space:]]*"id": "\([^"]*\)".*/\1/p' | head -n 1
}

"$planr_bin" project init "Portable Linux lifecycle" --json > project.json
"$planr_bin" project show --json > project-show.json
product_id="$("$planr_bin" plan new "Portable Linux lifecycle" --platform cli --json | first_id)"
test -n "$product_id"
build_id="$("$planr_bin" plan split "$product_id" --slice "Portable MVP" --json | first_id)"
test -n "$build_id"
"$planr_bin" map build --from "$build_id" --json > map-build.json
item_id="$("$planr_bin" pick --plan "$build_id" --work-type code --json | first_id)"
test -n "$item_id"
printf '%s\n' "portable lifecycle evidence" > evidence.txt
# The expanded executable precedes Planr's `done` subcommand; it is not the
# shell grammar keyword that static parsing sees here.
# shellcheck disable=SC1010
"$planr_bin" done "$item_id" \
  --summary "Verified the static Linux release lifecycle." \
  --files evidence.txt \
  --cmd "$planr_bin --version" \
  --tests "fresh lifecycle passed" \
  --json > done.json
"$planr_bin" map show --plan "$build_id" --json > map-show.json
grep -q '"status": "closed"' map-show.json
"$planr_bin" export \
  --out "$workspace/export" \
  --include-plans \
  --include-logs \
  --json > export.json
test -s "$workspace/export"
grep -q "$product_id" "$workspace/export"
test -f "$workspace/.planr/planr.sqlite"

echo "public_lifecycle=passed version=$expected_version project_plan_map_pick_done_export=true"
