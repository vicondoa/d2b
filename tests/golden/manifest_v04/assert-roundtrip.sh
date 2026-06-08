#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$(dirname "$(dirname "$HERE")")")}

# shellcheck source=../../lib.sh
. "$ROOT/tests/lib.sh"

baseline=${1:?baseline json required}
rendered=${2:?rendered json required}

if ! cmp -s "$baseline" "$rendered"; then
  diff -u "$baseline" "$rendered" >&2 || true
  fail "manifest-v04-roundtrip: rendered manifest differs from baseline"
fi

required_paths=(
  '["corp-vm","mtu"]'
  '["corp-vm","mssClamp"]'
  '["corp-vm","lan","allowEastWest"]'
  '["corp-vm","lan","effectiveEastWest"]'
  '["sys-work-net","mtu"]'
  '["sys-work-net","mssClamp"]'
  '["sys-work-net","lan","allowEastWest"]'
  '["sys-work-net","lan","effectiveEastWest"]'
)

for path_json in "${required_paths[@]}"; do
  if ! jq -e --argjson path "$path_json" 'any(paths(scalars); . == $path)' "$baseline" >/dev/null 2>&1; then
    fail "manifest-v04-roundtrip: canonical baseline is missing required networking path $path_json"
    exit 1
  fi
  if ! jq -e --argjson path "$path_json" 'any(paths(scalars); . == $path)' "$rendered" >/dev/null 2>&1; then
    fail "manifest-v04-roundtrip: rendered manifest dropped required networking path $path_json"
    exit 1
  fi

  baseline_value=$(jq -c --argjson path "$path_json" 'getpath($path)' "$baseline")
  rendered_value=$(jq -c --argjson path "$path_json" 'getpath($path)' "$rendered")
  if [ "$baseline_value" != "$rendered_value" ]; then
    fail "manifest-v04-roundtrip: networking field at path $path_json changed from $baseline_value to $rendered_value"
    exit 1
  fi
done

ok "manifest-v04-roundtrip: rendered output preserves all required networking fields"
