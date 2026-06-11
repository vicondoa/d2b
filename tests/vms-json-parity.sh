#!/usr/bin/env bash
# Public vms.json remains byte-compatible with v0.4.0.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
PLAN_MD=${NIXLING_PLAN_MD:-$ROOT/docs/adr/0026-native-signoz-observability.md}
BASE_COMMIT=${NIXLING_VMS_BASELINE_COMMIT:-91d69b0}
BASELINE_DIR=${BASELINE_DIR:-$ROOT/tests/golden}
BASELINE_FIXTURE=${BASELINE_FIXTURE:-$BASELINE_DIR/vms.json-$BASE_COMMIT}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

if [ ! -f packages/nixling-core/src/bundle.rs ] || [ ! -d docs/reference/schemas/v1 ]; then
  log "no W1 bundle schemas — skipping vms-json-parity (W1 unstaged)"
  exit 0
fi

render_vms_json() {
  local out=$2
  local cached
  # Follow-up: under static.sh parallelism, multiple gates compete on
  # the nix daemon when each runs its own nix-instantiate. Use the
  # shared smoke-render cache (already prewarmed serially by static.sh
  # before the parallel pool fires) instead of re-rendering.
  if cached=$(nl_smoke_vms_json 2>/dev/null) && [ -s "$cached" ]; then
    cp -f -- "$cached" "$out"
    return 0
  fi
  return 1
}

manifest_version() {
  jq -r '._manifest.manifestVersion // empty' "$1"
}

spec_correction_documented() {
  [ -f "$PLAN_MD" ] || return 1
  grep -qiE 'Spec corrections|Spec correction' "$PLAN_MD" \
    && grep -qiE 'vms\.json|manifestVersion' "$PLAN_MD"
}

scratch=$(nl_mktemp .vms-json-parity.XXXXXX)
current_json=$scratch/current-vms.json
diff_file=$scratch/vms.diff
add_cleanup "rm -rf -- \"$scratch\""

if ! render_vms_json "$ROOT" "$current_json"; then
  fail "vms-json-parity: could not render current vms.json"
fi

if [ ! -f "$BASELINE_FIXTURE" ]; then
  fail "vms-json-parity: baseline fixture $BASELINE_FIXTURE is missing"
fi

base_version=$(manifest_version "$BASELINE_FIXTURE")
current_version=$(manifest_version "$current_json")
if [ -z "$base_version" ] || [ -z "$current_version" ]; then
  fail "vms-json-parity: manifestVersion missing from current render or baseline fixture"
fi

baseline_json=$BASELINE_FIXTURE
if [ "$current_version" != "$base_version" ]; then
  if [ "$current_version" -lt "$base_version" ]; then
    fail "vms-json-parity: current manifestVersion $current_version is older than baseline $base_version"
  fi
  if ! spec_correction_documented; then
    fail "vms-json-parity: manifestVersion bump $base_version -> $current_version requires a Spec corrections row and a committed baseline fixture"
  fi

  matches=()
  shopt -s nullglob
  for candidate in "$BASELINE_DIR"/vms.json-*; do
    [ -f "$candidate" ] || continue
    if [ "$(manifest_version "$candidate")" = "$current_version" ]; then
      matches+=( "$candidate" )
    fi
  done
  shopt -u nullglob

  if [ "${#matches[@]}" -eq 0 ]; then
    fail "vms-json-parity: manifestVersion bump $base_version -> $current_version requires a committed tests/golden/vms.json-* baseline fixture"
  fi
  if [ "${#matches[@]}" -gt 1 ]; then
    printf '%s\n' "${matches[@]}" >&2
    fail "vms-json-parity: multiple baseline fixtures match manifestVersion $current_version"
  fi
  baseline_json=${matches[0]}
fi

if [ "$(manifest_version "$baseline_json")" != "$current_version" ]; then
  fail "vms-json-parity: baseline fixture $(basename "$baseline_json") does not match manifestVersion $current_version"
fi

if cmp -s "$baseline_json" "$current_json"; then
  ok "vms-json-parity: current vms.json matches baseline fixture $(basename "$baseline_json")"
  exit 0
fi

if diff -u "$baseline_json" "$current_json" > "$diff_file"; then
  :
fi
head -120 "$diff_file" >&2 || true
fail "vms-json-parity: rendered vms.json differs from baseline fixture $(basename "$baseline_json")"
