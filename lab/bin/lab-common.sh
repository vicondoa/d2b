#!/usr/bin/env bash
# Shared helpers for the window-chrome capture lab.
#
# Determinism strategy: the nested niri winit output takes whatever size the
# parent compositor gives it, so we do NOT rely on output size. Instead every
# guest window is made floating, given an exact size, and moved to an exact
# position. Captures are cropped to that known rect plus a fixed margin, so a
# capture is byte-comparable across runs and hosts.

set -uo pipefail

LAB_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LAB_OUT="${LAB_OUT:-$LAB_ROOT/out}"
LAB_CONFIG="$LAB_ROOT/config"
PROXY_BIN="${PROXY_BIN:-$LAB_ROOT/../packages/target/debug/d2b-wayland-proxy}"

# Hard image budget. Copilot rejects oversized attachments, so this is a gate,
# not a guideline: oversized files fail the capture instead of being sent.
IMG_MAX_BYTES="${IMG_MAX_BYTES:-5242880}"   # 5 MiB ceiling
IMG_WARN_BYTES="${IMG_WARN_BYTES:-2097152}" # 2 MiB target

lab_log() { printf '[lab] %s\n' "$*" >&2; printf '[lab] %s\n' "$*" >> "${LAB_OUT:-/tmp}/inner.log" 2>/dev/null; }
lab_die() { lab_log "FATAL: $*"; exit 1; }

# Render the pinned niri config from its template.
# lab_render_config <dest> [key=value ...]
lab_render_config() {
  local dest="$1"; shift
  local mode="${LAB_MODE:-1280x800}"
  local scale="${LAB_SCALE:-1}"
  local gaps="${LAB_GAPS:-16}"
  local border="${LAB_BORDER:-4}"
  local border_active="${LAB_BORDER_ACTIVE:-#7fc8ff}"
  local border_inactive="${LAB_BORDER_INACTIVE:-#505050}"
  local border_urgent="${LAB_BORDER_URGENT:-#9b0000}"
  local radius="${LAB_RADIUS:-8}"
  local clip="${LAB_CLIP:-false}"

  sed \
    -e "s|@@MODE@@|$mode|g" \
    -e "s|@@SCALE@@|$scale|g" \
    -e "s|@@GAPS@@|$gaps|g" \
    -e "s|@@BORDER@@|$border|g" \
    -e "s|@@BORDER_ACTIVE@@|$border_active|g" \
    -e "s|@@BORDER_INACTIVE@@|$border_inactive|g" \
    -e "s|@@BORDER_URGENT@@|$border_urgent|g" \
    -e "s|@@RADIUS@@|$radius|g" \
    -e "s|@@CLIP@@|$clip|g" \
    "$LAB_CONFIG/niri.kdl.in" > "$dest" || lab_die "config render failed"
  niri validate -c "$dest" >/dev/null 2>&1 || lab_die "rendered config invalid: $dest"
}

# PNG dimensions without external tooling.
# lab_png_size <file> -> "W H"
lab_png_size() {
  od -An -tu1 -j16 -N8 "$1" |
    awk '{printf "%d %d\n", $1*16777216+$2*65536+$3*256+$4, $5*16777216+$6*65536+$7*256+$8}'
}

# Enforce the image budget. Fails closed.
lab_check_image() {
  local f="$1"
  [[ -s "$f" ]] || lab_die "image missing or empty: $f"
  local bytes; bytes="$(stat -c %s "$f")"
  if (( bytes > IMG_MAX_BYTES )); then
    lab_die "image $f is ${bytes}B, over the ${IMG_MAX_BYTES}B ceiling"
  fi
  if (( bytes > IMG_WARN_BYTES )); then
    lab_log "WARN: $f is ${bytes}B, over the ${IMG_WARN_BYTES}B target"
  fi
  printf '%s\t%s\t%s\n' "$(basename "$f")" "$bytes" "$(lab_png_size "$f")" \
    >> "${LAB_SIZE_MANIFEST:-$LAB_OUT/sizes.tsv}"
}

# Wait for a window whose title matches, and echo its niri window id.
lab_wait_window() {
  local needle="$1" tries="${2:-80}" id
  for _ in $(seq 1 "$tries"); do
    id="$(niri msg -j windows 2>/dev/null |
      tr '}' '\n' |
      grep -F "$needle" |
      grep -oE '"id":[0-9]+' | head -1 | cut -d: -f2)"
    if [[ -n "$id" ]]; then
      printf '%s\n' "$id"
      return 0
    fi
    sleep 0.25
  done
  return 1
}

# Place a window at an exact floating rect so captures are reproducible.
# lab_place_window <id> <x> <y> <w> <h>
lab_place_window() {
  local id="$1" x="$2" y="$3" w="$4" h="$5"
  niri msg action move-window-to-floating --id "$id" >/dev/null 2>&1
  niri msg action set-window-width  --id "$id" "$w" >/dev/null 2>&1
  niri msg action set-window-height --id "$id" "$h" >/dev/null 2>&1
  # move-floating-window takes relative or absolute; absolute has no sign.
  niri msg action move-floating-window --id "$id" -x "$x" -y "$y" >/dev/null 2>&1
  sleep 0.4
}

# Start a proxy instance. Echoes the WAYLAND_DISPLAY name for guests.
# lab_start_proxy <tag> [extra proxy args...]
lab_start_proxy() {
  local tag="$1"; shift
  local sock="${XDG_RUNTIME_DIR:?}/wl-chromelab-$tag-$$"
  rm -f "$sock"
  "$PROXY_BIN" --listen "$sock" --connect "$WAYLAND_DISPLAY" "$@" \
    >"$LAB_OUT/proxy-$tag.log" 2>&1 &
  LAB_PROXY_PIDS+=("$!")
  local i
  for i in $(seq 1 60); do
    [[ -S "$sock" ]] && { printf '%s\n' "$(basename "$sock")"; return 0; }
    sleep 0.1
  done
  lab_log "proxy $tag failed to listen; log follows"
  cat "$LAB_OUT/proxy-$tag.log" >&2
  return 1
}

lab_cleanup() {
  local pid
  for pid in "${LAB_GUEST_PIDS[@]:-}"; do [[ -n "$pid" ]] && kill "$pid" 2>/dev/null; done
  for pid in "${LAB_PROXY_PIDS[@]:-}"; do [[ -n "$pid" ]] && kill "$pid" 2>/dev/null; done
  rm -f "${XDG_RUNTIME_DIR:?}"/wl-chromelab-*-$$
}

LAB_PROXY_PIDS=()
LAB_GUEST_PIDS=()
