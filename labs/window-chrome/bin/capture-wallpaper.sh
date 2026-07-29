#!/usr/bin/env bash
# Capture the tab over a colourful textured wallpaper, so the chrome is judged
# against a busy background rather than a flat fill.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"
trap 'lab_cleanup' EXIT
: "${LAB_OUT:?}"

SWAYBG="${SWAYBG:?swaybg path required}"
WALLPAPER="${WALLPAPER:-$LAB_OUT/wallpaper.png}"

capture() {
  local name="$1" x="$2" y="$3" w="$4" h="$5"
  grim -g "${x},${y} ${w}x${h}" "$LAB_OUT/$name.png" 2>>"$LAB_OUT/grim.log" \
    || lab_die "grim failed for $name"
  lab_check_image "$LAB_OUT/$name.png"
  lab_log "captured $name"
}

"$SWAYBG" -i "$WALLPAPER" -m fill >"$LAB_OUT/swaybg.log" 2>&1 &
LAB_GUEST_PIDS+=("$!")
sleep 1.5

run_case() {
  local name="$1" tag="$2" label="$3" accent="$4" bg="$5" fg="$6" body="$7"
  local display
  display="$(lab_start_proxy "$tag" \
    --vm-name "$tag" --border-enable --border-label "$label" \
    --border-color-active "$accent" --border-color-inactive "$accent")" \
    || lab_die "proxy $tag failed"
  WAYLAND_DISPLAY="$display" foot \
    --title "chrome lab $tag" \
    --override "colors.background=$bg" --override "colors.foreground=$fg" \
    --override "main.pad=14x14" \
    sh -c "$body" >"$LAB_OUT/guest-$tag.log" 2>&1 &
  LAB_GUEST_PIDS+=("$!")
  local id
  id="$(lab_wait_window "chrome lab $tag")" || {
    lab_log "$tag never mapped"; return 1
  }
  lab_place_window "$id" 170 140 700 360
  sleep 0.9
  capture "wall-$name" 110 80 820 470
  capture "wall-$name-detail" 140 110 480 90
}

run_case collapsed work "Work" "#ffb347" "101014" "d8d8e0" \
  'printf "\n  Over a textured wallpaper.\n\n"; sleep 3600'

for pid in "${LAB_GUEST_PIDS[@]:1}"; do kill "$pid" 2>/dev/null; done
for pid in "${LAB_PROXY_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
LAB_PROXY_PIDS=(); sleep 0.8

export D2B_CHROME_EXPANDED=1
run_case expanded work2 "Work" "#ffb347" "101014" "d8d8e0" \
  'printf "\n  Expanded over a textured wallpaper.\n\n"; sleep 3600'
unset D2B_CHROME_EXPANDED

echo WALL_OK > "$LAB_OUT/wall.status"
niri msg action quit --skip-confirmation >/dev/null 2>&1
