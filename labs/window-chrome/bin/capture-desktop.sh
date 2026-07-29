#!/usr/bin/env bash
# Capture the tab's states on the live desktop, into a timestamped run dir.
#
# Unlike the nested-niri harness, this runs against the operator's real
# compositor, so it shows the tab exactly as they see it: real border colours,
# real scale, real wallpaper behind the gaps.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB_ROOT="$(dirname "$HERE")"
cd "$LAB_ROOT" || exit 1
source "$HERE/capture-lib.sh"

RUN="${RUN_DIR:-out/live-$(date -u +%Y%m%d-%H%M%S)}"
mkdir -p "$RUN"

WIN_W="${WIN_W:-900}"
WIN_H="${WIN_H:-380}"

shoot() {
  local name="$1" expanded="$2" parts="$3" label="$4" accent="${5:-#ffb347}"

  "$HERE/stop-lab.sh"

  local envs=(RUST_LOG=warn WAYLAND_DISPLAY=wayland-1)
  [ "$expanded" = "1" ] && envs+=(D2B_CHROME_EXPANDED=1)
  [ -n "$parts" ] && envs+=("D2B_CHROME_PARTS=$LAB_ROOT/config/$parts")

  env "${envs[@]}" setsid "$HERE/d2b-chrome-lab" --label "$label" --accent "$accent" \
    -- foot --title "chrome lab $name" >/dev/null 2>&1 &

  local id
  id="$(wait_for_window "chrome lab $name")" || {
    echo "FAILED $name: window never mapped" >&2
    return 1
  }
  place_window "$id" "$WIN_W" "$WIN_H"

  if capture_window "$id" "$RUN/$name.png"; then
    echo "captured $name -> $RUN/$name.png"
  else
    echo "FAILED $name" >&2
    return 1
  fi
}

# Realms must be distinguishable at a glance, so each capture uses the accent
# that realm would actually carry rather than a shared placeholder.
shoot collapsed          0 parts-default.json Work                   '#ffb347'
shoot expanded-labelled  1 parts-default.json Work                   '#ffb347'
shoot expanded-compact   1 parts-compact.json Work                   '#ffb347'
shoot expanded-custom    1 parts-custom.json  Work                   '#ffb347'
shoot long-label         0 parts-default.json corp-workstation.work  '#ffb347'
shoot realm-personal     0 parts-default.json Personal               '#7fc8ff'
shoot realm-untrusted    0 parts-default.json Untrusted              '#f2557f'

# Narrow window: the labelled row cannot fit, so optional actions must yield
# from the end rather than the tab clipping or overhanging.
WIN_W=420 shoot narrow-overflow 1 parts-default.json Work '#ffb347'

"$HERE/stop-lab.sh"
echo "$RUN"
