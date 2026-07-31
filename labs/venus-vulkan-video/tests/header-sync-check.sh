#!/usr/bin/env bash
# Assert the venus-protocol headers vendored into the virglrenderer and Mesa
# forks are byte-identical to what the pinned venus-protocol revision
# generates.
#
# Why this exists: the forks vendor *generated* headers rather than running the
# generator at build time, so a change to the wire schema only reaches them
# when someone remembers to re-copy. Nothing failed when that was forgotten --
# the renderer simply kept decoding with the old rules while the driver encoded
# with the new ones, and every other check stayed green. A W1 panel reviewer
# found exactly that: a decode-side hardening landed in venus-protocol and the
# pinned renderer still had the unhardened decoder.
#
# Usage:
#   header-sync-check.sh [--fix]
#
# Environment:
#   VENUS_PROTOCOL_DIR   generator source (default: lab state clone)
#   VIRGL_DIR            virglrenderer fork
#   MESA_DIR             mesa fork
#   VENUS_LAB_PYTHON     python with mako

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VP_DIR="${VENUS_PROTOCOL_DIR:-$STATE/venus-protocol}"
VIRGL_DIR="${VIRGL_DIR:-$STATE/virglrenderer}"
MESA_DIR="${MESA_DIR:-$STATE/mesa}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

FIX=0
[ "${1:-}" = "--fix" ] && FIX=1

die() { echo "header-sync: $*" >&2; exit 1; }

[ -d "$VP_DIR" ] || die "no venus-protocol at $VP_DIR"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
mkdir -p "$out/r" "$out/d"

"$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$out/r" >/dev/null 2>&1 \
  || die "renderer generation failed"
"$PYTHON" "$VP_DIR/vn_protocol.py" --outdir "$out/d" >/dev/null 2>&1 \
  || die "driver generation failed"

status=0

# Compare one vendored tree against one generated tree. Only files that exist
# on BOTH sides are compared: the forks also vendor Vulkan headers and the
# vk_video directory, which the generator does not emit.
compare_tree() {
  local label=$1 src=$2 dst=$3
  [ -d "$dst" ] || { echo "header-sync: $label: no vendored dir at $dst" >&2; status=1; return; }

  local f base drift=0
  for f in "$src"/*.h; do
    base=$(basename "$f")
    # A generated header that is ABSENT from the vendored tree is drift, not a
    # file to skip. Skipping it meant deleting a vendored header entirely
    # passed this gate: the fork would build against an incomplete protocol
    # snapshot while the check reported "in sync".
    if [ ! -f "$dst/$base" ]; then
      if [ "$FIX" = 1 ]; then
        cp "$f" "$dst/$base"
        echo "header-sync: $label: added missing $base"
      else
        echo "header-sync: $label: $base is MISSING from the vendored tree" >&2
        drift=1
      fi
      continue
    fi
    if ! cmp -s "$f" "$dst/$base"; then
      if [ "$FIX" = 1 ]; then
        cp "$f" "$dst/$base"
        echo "header-sync: $label: updated $base"
      else
        echo "header-sync: $label: $base differs from the pinned generator" >&2
        drift=1
      fi
    fi
  done

  # Hand-written headers that ship alongside the generated ones. A list of one
  # today; kept as a loop because the next hand-written header should not have
  # to restructure this.
  # shellcheck disable=SC2043
  for base in vn_protocol_video_h264_flags.h; do
    [ -f "$VP_DIR/include/$base" ] || continue
    [ -f "$dst/$base" ] || continue
    if ! cmp -s "$VP_DIR/include/$base" "$dst/$base"; then
      if [ "$FIX" = 1 ]; then
        cp "$VP_DIR/include/$base" "$dst/$base"
        echo "header-sync: $label: updated $base"
      else
        echo "header-sync: $label: $base differs from venus-protocol" >&2
        drift=1
      fi
    fi
  done

  if [ "$drift" != 0 ]; then
    status=1
  else
    echo "header-sync: $label: in sync"
  fi
}

compare_tree virglrenderer "$out/r" "$VIRGL_DIR/src/venus/venus-protocol"
compare_tree mesa          "$out/d" "$MESA_DIR/src/virtio/venus-protocol"

if [ "$status" != 0 ]; then
  echo >&2
  echo "  The forks vendor generated headers instead of running the generator," >&2
  echo "  so a wire-schema change reaches them only by being copied. Drift here" >&2
  echo "  means the driver and the renderer disagree about the wire while every" >&2
  echo "  other check stays green. Rerun with --fix, then commit both forks." >&2
  exit 1
fi
