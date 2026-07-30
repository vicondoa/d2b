#!/usr/bin/env bash
# Outer driver: renders the pinned niri config, launches nested niri with an
# inner capture script, and reports results.
#
#   ./run-capture.sh <inner-script> [LAB_* env already exported]
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"

inner="${1:?usage: run-capture.sh <inner-script>}"
[[ -x "$LAB_ROOT/bin/$inner" || -f "$LAB_ROOT/bin/$inner" ]] || lab_die "no such inner script: $inner"

mkdir -p "$LAB_OUT"
rm -f "$LAB_OUT"/*.status "$LAB_OUT/sizes.tsv"

cfg="$LAB_OUT/niri-render.kdl"
lab_render_config "$cfg"
lab_log "config rendered: $cfg"

[[ -x "$PROXY_BIN" ]] || lab_die "proxy binary missing: $PROXY_BIN (cargo build -p d2b-wayland-proxy)"

timeout "${LAB_TIMEOUT:-180}" niri -c "$cfg" -- bash "$LAB_ROOT/bin/$inner" \
  > "$LAB_OUT/niri.log" 2>&1
rc=$?
lab_log "nested niri exited rc=$rc"

if compgen -G "$LAB_OUT/*.status" > /dev/null; then
  lab_log "status: $(cat "$LAB_OUT"/*.status | tr '\n' ' ')"
else
  lab_log "NO STATUS FILE - capture did not complete"
  tail -30 "$LAB_OUT/niri.log" >&2
  exit 1
fi

if [[ -f "$LAB_OUT/sizes.tsv" ]]; then
  lab_log "image manifest:"
  column -t "$LAB_OUT/sizes.tsv" >&2
fi
