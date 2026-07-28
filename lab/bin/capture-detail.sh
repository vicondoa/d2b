#!/usr/bin/env bash
# Detail captures: corner treatment, clip-to-geometry, and a CSD app whose own
# titlebar sits directly under the trusted band.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"
trap 'lab_cleanup' EXIT

: "${LAB_OUT:?}"
mkdir -p "$LAB_OUT"
PREFIX="${PREFIX:-detail}"

capture() {
  local name="$1" x="$2" y="$3" w="$4" h="$5"
  grim -g "${x},${y} ${w}x${h}" "$LAB_OUT/$name.png" 2>>"$LAB_OUT/grim.log" \
    || lab_die "grim failed for $name"
  lab_check_image "$LAB_OUT/$name.png"
  lab_log "captured $name"
}

start_guest() {
  local tag="$1" label="$2" accent="$3" title="$4" bg="$5" fg="$6" body="$7"
  local display
  display="$(lab_start_proxy "$tag" \
    --vm-name "$tag" --border-enable --border-label "$label" \
    --border-color-active "$accent" --border-color-inactive "$accent")" \
    || lab_die "proxy $tag failed"
  WAYLAND_DISPLAY="$display" foot \
    --title "$title" \
    --override "colors.background=$bg" --override "colors.foreground=$fg" \
    --override "main.pad=14x14" \
    sh -c "$body" >"$LAB_OUT/guest-$tag.log" 2>&1 &
  LAB_GUEST_PIDS+=("$!")
}

start_guest work "Work" "#ffa500" "chrome lab work" "101014" "d8d8e0" \
  'printf "\n  Corner and border detail.\n\n"; sleep 3600'

id="$(lab_wait_window "chrome lab work")" || {
  lab_log "guest never mapped"; cat "$LAB_OUT/guest-work.log" >&2; exit 1
}
lab_place_window "$id" 160 130 700 380
sleep 0.9

# Tight crops of the corners where niri's radius meets the band.
capture "$PREFIX-corner-topleft"  120  90 240 130
capture "$PREFIX-corner-topright" 620  90 280 130
# The full band with generous gap on both sides.
capture "$PREFIX-band-full"       120  90 780 120

# A status token, to see it in situ.
lab_log "restarting guest with a status token"
for pid in "${LAB_GUEST_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
for pid in "${LAB_PROXY_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
LAB_GUEST_PIDS=(); LAB_PROXY_PIDS=()
sleep 0.8

export D2B_CHROME_STATUS="MIC MUTED"
start_guest work2 "Work" "#ffa500" "chrome lab token" "101014" "d8d8e0" \
  'printf "\n  Capability token in situ.\n\n"; sleep 3600'
id2="$(lab_wait_window "chrome lab token")" || {
  lab_log "token guest never mapped"; cat "$LAB_OUT/guest-work2.log" >&2; exit 1
}
lab_place_window "$id2" 160 130 700 380
sleep 0.9
capture "$PREFIX-with-token" 120 90 780 120
unset D2B_CHROME_STATUS

echo DETAIL_OK > "$LAB_OUT/detail.status"
niri msg action quit --skip-confirmation >/dev/null 2>&1
