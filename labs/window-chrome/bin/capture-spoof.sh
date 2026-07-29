#!/usr/bin/env bash
# Adversarial capture: can a guest forge the identity tab?
#
# The tab is a security surface -- it is what an operator reads before deciding
# which realm to type a password into. So the question is not whether the proxy
# owns the pixels (it does, by construction), but whether a guest that controls
# every pixel *below* the band can produce something an operator would mistake
# for it.
#
# Two attacks are staged:
#
#   spoof-adjacent  a guest draws a pixel-matched fake tab immediately below
#                   the real one, claiming a different realm.
#   spoof-nested    a guest draws a fake window frame, complete with its own
#                   fake tab, inside its content area.
#
# Both are rendered by the guest with plain terminal output, which is the
# weakest possible attacker and therefore the fairest lower bound: anything a
# real toolkit could do, this can do better.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB_ROOT="$(dirname "$HERE")"
cd "$LAB_ROOT" || exit 1
source "$HERE/capture-lib.sh"

RUN="${RUN_DIR:-out/spoof-$(date -u +%Y%m%d-%H%M%S)}"
mkdir -p "$RUN"

WIN_W="${WIN_W:-900}"
WIN_H="${WIN_H:-300}"

shoot() {
  local name="$1" label="$2" accent="$3" body="$4"

  "$HERE/stop-lab.sh"

  env RUST_LOG=warn WAYLAND_DISPLAY=wayland-1 \
    "D2B_CHROME_PARTS=$LAB_ROOT/config/parts-default.json" \
    setsid "$HERE/d2b-chrome-lab" --label "$label" --accent "$accent" \
    -- foot --title "chrome lab $name" \
       --override "colors-dark.background=101014" \
       --override "colors-dark.foreground=d8d8e0" \
       --override "main.pad=0x0" \
       sh -c "$body" >/dev/null 2>&1 &

  local id
  id="$(wait_for_window "chrome lab $name")" || {
    echo "FAILED $name: window never mapped" >&2
    return 1
  }
  place_window "$id" "$WIN_W" "$WIN_H"
  sleep 0.6

  if capture_window "$id" "$RUN/$name.png"; then
    echo "captured $name -> $RUN/$name.png"
  else
    echo "FAILED $name" >&2
    return 1
  fi
}

# 1. A fake tab flush under the real one, claiming a different realm.
#    The real window is Work (orange). The guest claims Personal (blue).
spoof_adjacent_body=$(cat <<'GUEST'
printf '\033[48;2;127;200;255m \033[48;2;37;39;43m \033[1;37mPersonal \033[0;38;2;208;208;208m>\033[0m\n'
printf '\n  A guest drew the bar directly above this line.\n'
printf '  The real, proxy-owned tab is the one further up.\n\n'
sleep 3600
GUEST
)

# 2. A fake nested window, frame and all, inside the guest content.
spoof_nested_body=$(cat <<'GUEST'
printf '\n'
printf '  \033[38;2;219;183;255m┌────────────────────────────────────────────┐\033[0m\n'
printf '  \033[38;2;219;183;255m│\033[0m \033[48;2;127;200;255m \033[48;2;37;39;43m \033[1;37mPersonal \033[0;38;2;208;208;208m>\033[0m                       \033[38;2;219;183;255m│\033[0m\n'
printf '  \033[38;2;219;183;255m│\033[0m                                            \033[38;2;219;183;255m│\033[0m\n'
printf '  \033[38;2;219;183;255m│\033[0m  Enter your password:                      \033[38;2;219;183;255m│\033[0m\n'
printf '  \033[38;2;219;183;255m│\033[0m  ________________________                  \033[38;2;219;183;255m│\033[0m\n'
printf '  \033[38;2;219;183;255m│\033[0m                                            \033[38;2;219;183;255m│\033[0m\n'
printf '  \033[38;2;219;183;255m└────────────────────────────────────────────┘\033[0m\n'
sleep 3600
GUEST
)

shoot spoof-adjacent Work '#ffb347' "$spoof_adjacent_body"
shoot spoof-nested   Work '#ffb347' "$spoof_nested_body"

"$HERE/stop-lab.sh"
echo "$RUN"
