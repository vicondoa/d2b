#!/usr/bin/env bash
# Shared capture helpers for the live-desktop lab.
#
# Two things here exist because of bugs that produced *plausible but wrong*
# screenshots, which is worse than an error:
#
#  - Window placement acts on an explicit window id. Acting on "the focused
#    window" resized whatever the operator happened to be using, because the
#    lab window does not reliably take focus when it maps.
#  - Crops are derived from the window's reported rect, not a hardcoded one. A
#    fixed rect silently captured an unrelated application the moment placement
#    went wrong, and the resulting image looked entirely reasonable.
set -uo pipefail

# grim works in physical output pixels; niri reports logical coordinates.
niri_scale() {
  niri msg -j outputs | jq -r 'to_entries[0].value.logical.scale // 1'
}

# Window id for an exact title, or empty.
window_id_for_title() {
  local title="$1"
  niri msg -j windows \
    | jq -r --arg t "[chromelab] $title" 'map(select(.title == $t)) | .[0].id // empty'
}

# Wait for a window with the given title to map, then echo its id.
wait_for_window() {
  local title="$1" tries="${2:-40}" id
  for _ in $(seq 1 "$tries"); do
    id="$(window_id_for_title "$title")"
    if [ -n "$id" ]; then
      printf '%s\n' "$id"
      return 0
    fi
    sleep 0.4
  done
  return 1
}

# Float and size a window BY ID.
place_window() {
  local id="$1" w="$2" h="$3"
  niri msg action focus-window --id "$id" >/dev/null 2>&1
  sleep 0.4
  niri msg action move-window-to-floating --id "$id" >/dev/null 2>&1
  sleep 0.4
  niri msg action set-window-width --id "$id" "$w" >/dev/null 2>&1
  niri msg action set-window-height --id "$id" "$h" >/dev/null 2>&1
  sleep 1.2
}

# Echo the window's on-screen rect in PHYSICAL px as "x,y WxH".
#
# Only used as a fallback. `tile_pos_in_workspace_view` is in workspace-view
# coordinates, which do not account for the output's position or for scrolling
# offsets, so this is not reliable on a multi-output or scrolled workspace.
# Prefer capture_window, which asks the compositor for the window directly.
window_rect() {
  local id="$1" pad="${2:-12}" scale
  scale="$(niri_scale)"
  niri msg -j windows | jq -r \
    --argjson id "$id" --argjson pad "$pad" --argjson scale "$scale" '
      (map(select(.id == $id)) | .[0]) as $w
      | if $w == null or $w.layout.tile_pos_in_workspace_view == null then
          empty
        else
          ([$w.layout.tile_pos_in_workspace_view[0] - $pad, 0] | max) as $x
          | ([$w.layout.tile_pos_in_workspace_view[1] - $pad, 0] | max) as $y
          | ($w.layout.tile_size[0] + $pad * 2) as $tw
          | ($w.layout.tile_size[1] + $pad * 2) as $th
          | "\($x * $scale | floor),\($y * $scale | floor) \($tw * $scale | floor)x\($th * $scale | floor)"
        end'
}

# Capture a window by id into a file.
#
# Uses niri's own screenshot-window action rather than cropping a full-screen
# grab to computed coordinates. Computing the rect from
# `tile_pos_in_workspace_view` produced images of *other applications* when the
# arithmetic was off, which is the worst kind of failure: the screenshot looks
# perfectly reasonable and is simply not the thing under review. Asking the
# compositor which pixels belong to a window id cannot make that mistake, and
# has the side benefit of excluding everything the window does not own.
capture_window() {
  local id="$1" out="$2" bytes before after
  local shotdir="${NIRI_SCREENSHOT_DIR:-$HOME/Pictures/Screenshots}"

  before="$(ls -1t "$shotdir" 2>/dev/null | head -1)"
  if ! niri msg action screenshot-window --id "$id" >/dev/null 2>&1; then
    echo "niri screenshot-window failed for window $id" >&2
    return 1
  fi

  # The action writes asynchronously.
  for _ in $(seq 1 25); do
    after="$(ls -1t "$shotdir" 2>/dev/null | head -1)"
    [ -n "$after" ] && [ "$after" != "$before" ] && break
    sleep 0.2
  done
  if [ -z "${after:-}" ] || [ "$after" = "$before" ]; then
    echo "no screenshot appeared in $shotdir for window $id" >&2
    return 1
  fi

  # Move rather than copy, so the lab does not litter the operator's
  # screenshot folder with build artifacts.
  mv "$shotdir/$after" "$out" || return 1

  # The image budget is a hard gate: an oversized file blocks rather than
  # degrades, because Copilot fails on oversized attachments.
  bytes="$(stat -c%s "$out")"
  if [ "$bytes" -gt 5000000 ]; then
    echo "image $out is ${bytes} bytes, over the 5 MB budget" >&2
    return 1
  fi
  return 0
}
