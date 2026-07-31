#!/usr/bin/env bash
#
# The generated rejection table must match what the generator produces.
#
# Without this, a stale vkr_video_reject.h is indistinguishable from a current
# one: the enforcement gate credits a site when the type and member NAMES are
# reachable, so a helper whose value mask has fallen behind the manifest still
# counts as enforced. That is the difference between "a check exists" and "the
# check is the right one", and only regeneration can tell them apart.
#
# Usage: generator-drift-check.sh

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VIRGL_DIR="${VIRGL_DIR:-$STATE/virglrenderer}"
VP_DIR="${VENUS_PROTOCOL_DIR:-$STATE/venus-protocol}"
MANIFEST="${VIDEO_SITE_MANIFEST:-$HERE/video-site-manifest-golden.txt}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"
GENERATOR="${VIDEO_REJECT_GENERATOR:-$HERE/gen-video-reject.py}"

die() { echo "generator-drift: $*" >&2; exit 1; }

vendored="$VIRGL_DIR/src/venus/vkr_video_reject.h"
[ -f "$vendored" ] || die "no vendored header at $vendored"
[ -f "$VP_DIR/xmls/vk.xml" ] || die "no vk.xml under $VP_DIR"
[ -f "$MANIFEST" ] || die "no site manifest at $MANIFEST"
[ -f "$GENERATOR" ] || die "no generator at $GENERATOR"

tmp=$(mktemp)

log=$(mktemp)
trap 'rm -f "$tmp" "$log"' EXIT

if ! "$PYTHON" "$GENERATOR" "$MANIFEST" \
     "$VP_DIR/xmls/vk.xml" \
     "$VIRGL_DIR/src/venus/venus-protocol/vn_protocol_renderer_descriptor_heap.h" \
     "$VIRGL_DIR/src/venus/venus-protocol/vn_protocol_renderer_device.h" \
     >"$tmp" 2>"$log"; then
  echo "generator-drift: generator failed" >&2
  cat "$log" >&2
  exit 1
fi

if ! diff -u "$vendored" "$tmp" >/dev/null; then
  echo "generator-drift: FAIL -- vendored header differs from generator output" >&2
  echo >&2
  diff -u "$vendored" "$tmp" | head -40 >&2
  echo >&2
  echo "  Regenerate with:" >&2
  echo "    gen-video-reject.py <manifest> <vk.xml> > src/venus/vkr_video_reject.h" >&2
  exit 1
fi

echo "generator-drift: PASS -- vendored header matches the generator"
