#!/usr/bin/env bash
# Live in-situ capture: the identity band drawn by the REAL proxy, inside niri,
# with niri's own border, gaps, and rounded corners around it.
#
# Runs inside the nested niri (launched as its startup command).
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"
trap 'lab_cleanup' EXIT

: "${LAB_OUT:?}"
mkdir -p "$LAB_OUT"

MARGIN="${MARGIN:-46}"

capture() {
  local name="$1" x="$2" y="$3" w="$4" h="$5"
  grim -g "${x},${y} ${w}x${h}" "$LAB_OUT/$name.png" 2>>"$LAB_OUT/grim.log" \
    || lab_die "grim failed for $name"
  lab_check_image "$LAB_OUT/$name.png"
  lab_log "captured $name ($(stat -c %s "$LAB_OUT/$name.png")B)"
}

# Capture a window plus the compositor gap around it, so niri's border and
# corner radius are part of the evidence rather than cropped away.
capture_window() {
  local name="$1" id="$2"
  local geo
  geo="$(niri msg -j windows 2>/dev/null \
    | tr '{' '\n' | grep "\"id\":$id," \
    | grep -oE '"tile_pos_in_workspace_view":\[[0-9.]+,[0-9.]+\],"[^"]*":' )"
  # Positions come from our own placement, so use those directly.
  local x="$3" y="$4" w="$5" h="$6"
  local cx=$(( x - MARGIN )); local cy=$(( y - MARGIN ))
  (( cx < 0 )) && cx=0
  (( cy < 0 )) && cy=0
  capture "$name" "$cx" "$cy" "$(( w + MARGIN * 2 ))" "$(( h + MARGIN * 2 ))"
}

start_guest() {
  local tag="$1" label="$2" accent="$3" title="$4" bg="$5" fg="$6"; shift 6
  local display
  display="$(lab_start_proxy "$tag" \
    --vm-name "$tag" \
    --border-enable \
    --border-label "$label" \
    --border-color-active "$accent" \
    --border-color-inactive "$accent")" || lab_die "proxy $tag failed"

  WAYLAND_DISPLAY="$display" foot \
    --title "$title" \
    --override "colors.background=$bg" \
    --override "colors.foreground=$fg" \
    --override "main.pad=14x14" \
    sh -c "$1" \
    >"$LAB_OUT/guest-$tag.log" 2>&1 &
  LAB_GUEST_PIDS+=("$!")
}

body() {
  printf 'printf "\\n  %s\\n\\n  %s\\n\\n"; sleep 3600' "$1" "$2"
}

# --- 1. single window, dark content -----------------------------------------
start_guest work "Work" "#ffa500" "chrome lab work" "101014" "d8d8e0" \
  "$(body 'Identity band drawn by the real proxy.' 'niri draws its border around the band-inclusive rect.')"

id_work="$(lab_wait_window "chrome lab work")" || {
  lab_log "work guest never mapped"; cat "$LAB_OUT/guest-work.log" >&2; exit 1
}
lab_place_window "$id_work" 140 110 860 470
sleep 0.8
capture_window "live-01-focused" "$id_work" 140 110 860 470

# Unfocused: start a second window in another realm and focus it.
start_guest personal "Personal" "#7fc8ff" "chrome lab personal" "0f1418" "cfe6f5" \
  "$(body 'A second realm.' 'Two identities side by side on one screen.')"

id_personal="$(lab_wait_window "chrome lab personal")" || {
  lab_log "personal guest never mapped"; cat "$LAB_OUT/guest-personal.log" >&2; exit 1
}
lab_place_window "$id_personal" 140 620 860 340
sleep 0.8

# Both visible at once, with the work window now unfocused.
capture "live-02-two-realms" 90 60 970 960

# Focus back on work and capture the pair from the other side.
niri msg action focus-window --id "$id_work" >/dev/null 2>&1
sleep 0.6
capture "live-03-focus-moved" 90 60 970 960

# --- 2. light guest content --------------------------------------------------
start_guest media "Media" "#c792ea" "chrome lab media" "f4f4f8" "1c1c22" \
  "$(body 'Light application content.' 'The band must hold its own against a bright window.')"

id_media="$(lab_wait_window "chrome lab media")" || {
  lab_log "media guest never mapped"; cat "$LAB_OUT/guest-media.log" >&2; exit 1
}
lab_place_window "$id_media" 140 110 860 470
sleep 0.8
capture_window "live-04-light-content" "$id_media" 140 110 860 470

# --- 3. narrow window --------------------------------------------------------
lab_place_window "$id_media" 140 110 300 470
sleep 0.8
capture_window "live-05-narrow" "$id_media" 140 110 300 470

# --- 4. fullscreen -----------------------------------------------------------
niri msg action focus-window --id "$id_work" >/dev/null 2>&1
sleep 0.3
niri msg action fullscreen-window --id "$id_work" >/dev/null 2>&1
sleep 1.0
out_w="$(niri msg -j outputs 2>/dev/null | grep -oE '"logical":\{[^}]*"width":[0-9]+' | grep -oE '[0-9]+$' | head -1)"
out_h="$(niri msg -j outputs 2>/dev/null | grep -oE '"logical":\{[^}]*"height":[0-9]+' | grep -oE '[0-9]+$' | head -1)"
capture "live-06-fullscreen" 0 0 "${out_w:-1200}" "${out_h:-800}"
niri msg action fullscreen-window --id "$id_work" >/dev/null 2>&1
sleep 0.6

niri msg -j windows > "$LAB_OUT/windows-live.json"
echo LIVE_OK > "$LAB_OUT/live.status"
niri msg action quit --skip-confirmation >/dev/null 2>&1
