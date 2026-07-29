#!/usr/bin/env bash
# Baseline capture: the CURRENT 9px rail, as shipped, so every prototype has a
# like-for-like "before" to be judged against.
#
# Runs inside the nested niri (launched as its startup command).
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"
trap 'lab_cleanup' EXIT

: "${LAB_OUT:?}"
mkdir -p "$LAB_OUT"

WIN_X="${WIN_X:-120}"
WIN_Y="${WIN_Y:-90}"
WIN_W="${WIN_W:-880}"
WIN_H="${WIN_H:-560}"
MARGIN="${MARGIN:-40}"

capture() {
  local name="$1" x="$2" y="$3" w="$4" h="$5"
  # grim rejects -o together with -g; the nested output sits at 0,0 so output
  # coordinates and the crop rect share one space.
  grim -g "${x},${y} ${w}x${h}" "$LAB_OUT/$name.png" 2>>"$LAB_OUT/grim.log" \
    || lab_die "grim failed for $name"
  lab_check_image "$LAB_OUT/$name.png"
  lab_log "captured $name ($(stat -c %s "$LAB_OUT/$name.png")B)"
}

display="$(lab_start_proxy baseline \
  --vm-name work \
  --border-enable \
  --border-label "${LAB_LABEL:-work}" \
  --border-color-active "${LAB_ACCENT:-#ffa500}" \
  --border-color-inactive "${LAB_ACCENT:-#ffa500}")" || lab_die "proxy start failed"

WAYLAND_DISPLAY="$display" foot \
  --title "chrome lab guest" \
  --override "colors.background=101014" \
  --override "colors.foreground=d8d8e0" \
  --override "main.pad=12x12" \
  sh -c 'printf "\n  Baseline: current 9px rail\n\n  This window belongs to a VM.\n  Which one? Read the rail.\n\n"; sleep 3600' \
  >"$LAB_OUT/guest.log" 2>&1 &
LAB_GUEST_PIDS+=("$!")

id="$(lab_wait_window "chrome lab guest")" || {
  lab_log "guest never mapped"; cat "$LAB_OUT/guest.log" >&2; exit 1
}
lab_log "guest window id=$id"
lab_place_window "$id" "$WIN_X" "$WIN_Y" "$WIN_W" "$WIN_H"

niri msg -j windows > "$LAB_OUT/windows-baseline.json"

cx=$(( WIN_X - MARGIN )); cy=$(( WIN_Y - MARGIN ))
cw=$(( WIN_W + MARGIN * 2 )); ch=$(( WIN_H + MARGIN * 2 ))
(( cx < 0 )) && cx=0
(( cy < 0 )) && cy=0

sleep 0.6
capture "baseline-rail-focused" "$cx" "$cy" "$cw" "$ch"

# Detail crop of the identity affordance itself: the question is the rail, not
# the terminal contents.
capture "baseline-rail-detail" "$cx" "$cy" "$(( MARGIN * 2 + 120 ))" "$ch"

echo BASELINE_OK > "$LAB_OUT/baseline.status"
niri msg action quit --skip-confirmation >/dev/null 2>&1
