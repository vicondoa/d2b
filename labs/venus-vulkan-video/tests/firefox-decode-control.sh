#!/usr/bin/env bash
# A/B negative control for the Firefox Vulkan decode claim.
#
# The positive result -- a renderer context appearing with hundreds of
# vkCmdDecodeVideoKHR calls while Firefox plays -- only means something if the
# SAME playback produces none of them when the decoder is switched off.
# Otherwise the counter could be attributing another process's decode to
# Firefox, or counting something that happens regardless.
#
# Measures the renderer's own decode counter across two runs of identical
# length against an identical clip, differing only in
# media.hardware-video-decoding-vulkan.enabled.
#
# That pref must NOT be policy-locked. It was, on the first attempt: the write
# was silently refused, the control appeared to run, playback continued, and
# the result read as "decode happens either way" -- the exact false pass this
# script exists to rule out.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
LAB="git+file://$ROOT?dir=labs/venus-vulkan-video"
LOG="${VENUS_LAB_LAUNCHER_LOG:-${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/launcher.log}"
EVIDENCE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/evidence"

# Total decode commands across all renderer contexts.
#
# The counter is a file-static in the render-server process, so every context
# starts again at 1. A global max is therefore WRONG: once one context has
# reported 2048, a fresh context climbing to 512 does not move it, and a real
# positive reads as delta 0. Measured that exact false negative while restoring
# the positive phase. Sum the per-context maxima instead.
decode_total() {
  awk 'match($0, /virgl_render_server\[[0-9]+\]/) {
         ctx = substr($0, RSTART, RLENGTH)
       }
       match($0, /decode_cmds=[0-9]+/) {
         n = substr($0, RSTART + 12, RLENGTH - 12) + 0
         if (n > m[ctx]) m[ctx] = n
       }
       END { t = 0; for (k in m) t += m[k]; print t }' "$LOG" 2>/dev/null || echo 0
}
session_total() {
  grep -c 'VIDEO-EVIDENCE session created' "$LOG" 2>/dev/null || echo 0
}

run_phase() {
  local state="$1"
  nix run "$LAB#lab-ssh" -- --stdin 'python3 - '"$state" \
    < "$ROOT/labs/venus-vulkan-video/guest/firefox-negative-control.py"
}

mkdir -p "$EVIDENCE"
printf '=== A/B negative control: Firefox Vulkan decode ===\n'

for state in on off on; do
  d0=$(decode_total); s0=$(session_total)
  printf '\n--- pref=%s ---\n' "$state"
  run_phase "$state" 2>&1 | sed 's/^/  /'
  sleep 3
  d1=$(decode_total); s1=$(session_total)
  printf '  renderer decode_cmds: %s -> %s (delta %s)\n' "$d0" "$d1" "$((d1 - d0))"
  printf '  renderer sessions:    %s -> %s (delta %s)\n' "$s0" "$s1" "$((s1 - s0))"
  printf 'PHASE %s decode_delta=%s session_delta=%s\n' \
    "$state" "$((d1 - d0))" "$((s1 - s0))"
done

printf '\nInterpretation: pref=on must show a nonzero session delta and a rising\n'
printf 'decode count; pref=off must show zero of both while the video still\n'
printf 'plays. Playback continuing under pref=off is expected -- that is the\n'
printf 'software fallback, and it is why frame counts prove nothing on their own.\n'
