#!/usr/bin/env bash
# Smoke test: nested niri + real d2b-wayland-proxy + foot stand-in guest + grim capture.
# Runs INSIDE the nested niri (niri launches it as its startup command), so
# WAYLAND_DISPLAY and NIRI_SOCKET point at the nested compositor.
set -uo pipefail

LAB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$LAB/out"
PROXY="$LAB/../packages/target/debug/d2b-wayland-proxy"
SOCK_DIR="${XDG_RUNTIME_DIR:?}"
PROXY_SOCK="$SOCK_DIR/wl-chromelab-$$"

log() { printf '[smoke] %s\n' "$*" >&2; }

cleanup() {
  [[ -n "${FOOT_PID:-}" ]] && kill "$FOOT_PID" 2>/dev/null
  [[ -n "${PROXY_PID:-}" ]] && kill "$PROXY_PID" 2>/dev/null
  rm -f "$PROXY_SOCK"
}
trap cleanup EXIT

log "nested WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-unset} NIRI_SOCKET=${NIRI_SOCKET:-unset}"

"$PROXY" \
  --listen "$PROXY_SOCK" \
  --connect "$WAYLAND_DISPLAY" \
  --vm-name work \
  --border-enable \
  --border-label "WORK" \
  --border-color-active "#ffa500" \
  >"$OUT/proxy.log" 2>&1 &
PROXY_PID=$!

for _ in $(seq 1 50); do
  [[ -S "$PROXY_SOCK" ]] && break
  sleep 0.1
done
if [[ ! -S "$PROXY_SOCK" ]]; then
  log "FAIL: proxy socket never appeared"
  cat "$OUT/proxy.log" >&2
  exit 1
fi
log "proxy socket up"

WAYLAND_DISPLAY="$(basename "$PROXY_SOCK")" \
  foot --title "chrome lab guest" \
  >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!

# Wait for the guest window to actually map in the nested compositor.
mapped=0
for _ in $(seq 1 80); do
  if niri msg -j windows 2>/dev/null | grep -q 'chrome lab guest'; then
    mapped=1
    break
  fi
  sleep 0.25
done
if [[ "$mapped" != 1 ]]; then
  log "FAIL: guest window never mapped"
  log "--- proxy.log ---"; cat "$OUT/proxy.log" >&2
  log "--- foot.log ---"; cat "$OUT/foot.log" >&2
  exit 1
fi
log "guest window mapped"

niri msg -j windows > "$OUT/windows.json" 2>/dev/null
sleep 1
grim -o winit "$OUT/smoke.png" 2>>"$OUT/grim.log"
rc=$?
if [[ $rc -ne 0 || ! -s "$OUT/smoke.png" ]]; then
  log "FAIL: grim capture failed (rc=$rc)"
  cat "$OUT/grim.log" >&2
  exit 1
fi
log "captured $(stat -c %s "$OUT/smoke.png") bytes"
echo SMOKE_OK > "$OUT/smoke.status"
