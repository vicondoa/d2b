#!/usr/bin/env bash
# Live capture of the compact tab: collapsed, expanded with action icons, and
# with a capability token, all inside niri.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"
trap 'lab_cleanup' EXIT

: "${LAB_OUT:?}"
mkdir -p "$LAB_OUT"

capture() {
  local name="$1" x="$2" y="$3" w="$4" h="$5"
  grim -g "${x},${y} ${w}x${h}" "$LAB_OUT/$name.png" 2>>"$LAB_OUT/grim.log" \
    || lab_die "grim failed for $name"
  lab_check_image "$LAB_OUT/$name.png"
  lab_log "captured $name"
}

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
    lab_log "$tag never mapped"; cat "$LAB_OUT/guest-$tag.log" >&2; return 1
  }
  lab_place_window "$id" 160 130 720 380
  sleep 0.9
  # Full window with gap, then a tight crop of just the tab.
  capture "tab-$name" 110 80 820 480
  capture "tab-$name-detail" 130 100 460 90
  printf '%s\n' "$id"
}

# 1. collapsed
run_case collapsed work "Work" "#ffa500" "101014" "d8d8e0" \
  'printf "\n  Compact tab, transparent surround.\n\n"; sleep 3600' >/dev/null

for pid in "${LAB_GUEST_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
for pid in "${LAB_PROXY_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
LAB_GUEST_PIDS=(); LAB_PROXY_PIDS=(); sleep 0.8

# 2. expanded with action icons
export D2B_CHROME_EXPANDED=1
run_case expanded work2 "Work" "#ffa500" "101014" "d8d8e0" \
  'printf "\n  Tab expanded: actions sit beside the name.\n\n"; sleep 3600' >/dev/null
unset D2B_CHROME_EXPANDED

for pid in "${LAB_GUEST_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
for pid in "${LAB_PROXY_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
LAB_GUEST_PIDS=(); LAB_PROXY_PIDS=(); sleep 0.8

# 3. light content, different realm
run_case light media "Media" "#c792ea" "f4f4f8" "1c1c22" \
  'printf "\n  Light application content.\n\n"; sleep 3600' >/dev/null

for pid in "${LAB_GUEST_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
for pid in "${LAB_PROXY_PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
LAB_GUEST_PIDS=(); LAB_PROXY_PIDS=(); sleep 0.8

# 4. with a capability token
export D2B_CHROME_STATUS="MIC MUTED"
run_case token personal "Personal" "#7fc8ff" "0f1418" "cfe6f5" \
  'printf "\n  Capability token present.\n\n"; sleep 3600' >/dev/null
unset D2B_CHROME_STATUS

echo TAB_OK > "$LAB_OUT/tab.status"
niri msg action quit --skip-confirmation >/dev/null 2>&1
