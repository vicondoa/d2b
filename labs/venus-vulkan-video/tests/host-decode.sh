#!/usr/bin/env bash
# host-decode.sh - prove the HOST can actually decode H.264 via Vulkan Video.
#
# Why this exists as a separate, mandatory W0 gate:
#
#   Host `vulkaninfo` showing VK_KHR_video_decode_h264 proves the driver
#   ADVERTISES the extension. It does not prove a decode actually runs. If that
#   distinction is left unmeasured, a failure in the guest (W5) is ambiguous:
#   it could be a Venus forwarding bug, or the host path could have been broken
#   all along. This gate removes that ambiguity before any protocol work starts.
#
# The same false-pass discipline as the guest applies, and it matters here:
#
#   `ffmpeg -hwaccel vulkan ... -f null -` exits **0** while silently falling
#   back to software when hwaccel init fails. Exit status alone is worthless.
#   Attribution comes from the verbose log.
set -euo pipefail

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab"
OUT_DIR="${VENUS_LAB_OUT:-$STATE_DIR/evidence}"
CLIP="$OUT_DIR/h264-baseline.mp4"
LOG="$OUT_DIR/host-decode.log"

have() { command -v "$1" >/dev/null 2>&1; }

ffmpeg_cmd() {
  # The host-vs-guest comparison is only a controlled experiment if BOTH sides
  # run the same ffmpeg build. The guest uses the lab flake's pinned ffmpeg, so
  # the host must too. VENUS_LAB_FFMPEG is set by the `host-decode` flake app to
  # the pinned store path; falling back to an ambient or registry-resolved
  # ffmpeg would silently compare two different builds.
  if [ -n "${VENUS_LAB_FFMPEG:-}" ]; then
    echo "$VENUS_LAB_FFMPEG"
    return
  fi
  echo "error: VENUS_LAB_FFMPEG is not set." >&2
  echo "  Run this through the flake so the pinned ffmpeg is used:" >&2
  echo "    nix run '.#host-decode'" >&2
  echo "  Using an ambient ffmpeg would invalidate the host-vs-guest control." >&2
  exit 1
}

main() {
  mkdir -p "$OUT_DIR"
  local ff; ff=$(ffmpeg_cmd)

  echo "=== host Vulkan Video decode baseline ==="

  # Deterministic local clip: H.264 High, 8-bit, 4:2:0, progressive -- exactly
  # the profile the prototype targets. Generated, not downloaded, so the result
  # is reproducible offline and identical to the guest's corpus.
  if [ ! -f "$CLIP" ]; then
    echo "generating deterministic H.264 clip"
    $ff -y -loglevel error \
      -f lavfi -i testsrc=size=1280x720:rate=30:duration=3 \
      -pix_fmt yuv420p -c:v libx264 -profile:v high "$CLIP"
  fi
  echo "clip: $CLIP ($(stat -c %s "$CLIP") bytes)"

  echo "--- hwaccels advertised by this ffmpeg build ---"
  # Note: 'vulkan' appearing here proves only that the BUILD supports it. It is
  # not evidence that a decode works, and treating it as such would be another
  # false pass.
  $ff -hide_banner -hwaccels 2>/dev/null | tail -n +2 | tr '\n' ' '
  echo

  echo "--- attempting Vulkan decode ---"
  local exit_status=0
  $ff -hide_banner -loglevel verbose -y \
      -hwaccel vulkan -hwaccel_output_format vulkan \
      -i "$CLIP" -f null - > "$LOG" 2>&1 || exit_status=$?
  echo "exit_status=$exit_status  (NOT sufficient evidence on its own)"

  local init fell pixfmt
  init=$(grep -ciE "Init(ialized)? .*vulkan|using vulkan|vulkan_decode" "$LOG" || true)
  fell=$(grep -ciE "Failed setup for format vulkan|falling back|not supported|No device available" "$LOG" || true)
  # The decisive signal: the decoder's OUTPUT pixel format. `pix_fmt: vulkan`
  # means frames are Vulkan-backed; `pix_fmt: yuv420p` means it decoded in
  # software regardless of what was requested. This discriminates far more
  # reliably than log phrasing, which varies between ffmpeg versions.
  pixfmt=$(grep -oE "pix_fmt: [a-z0-9]+" "$LOG" | tail -1 | awk '{print $2}' || true)
  echo "vulkan_init_lines=$init"
  echo "vulkan_fallback_lines=$fell"
  echo "decoder_output_pix_fmt=${pixfmt:-unknown}"

  echo "--- relevant log lines ---"
  grep -iE "vulkan|hwaccel|decoder|h264" "$LOG" 2>/dev/null | head -10 | sed 's/^/  /' || true
  echo
  echo "full log: $LOG"

  if [ "$exit_status" -eq 0 ] && [ "$pixfmt" = "vulkan" ] && [ "$fell" -eq 0 ]; then
    echo "RESULT: host DID decode H.264 through Vulkan Video (pix_fmt=vulkan)"
    return 0
  fi
  echo "RESULT: host did NOT use Vulkan Video for decode" >&2
  echo "  This is a blocker for the whole prototype: if the host cannot decode" >&2
  echo "  natively, forwarding it through Venus cannot work either. Investigate" >&2
  echo "  before starting protocol work." >&2
  return 1
}

case "${1:-}" in
  -h|--help) sed -n '2,20p' "$0" ;;
  "") main ;;
  *) echo "usage: ${0##*/}" >&2; exit 2 ;;
esac
