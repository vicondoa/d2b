#!/usr/bin/env bash
# Assert the virtio-gpu Venus capset does not advertise video.
#
# Door 9, found by the virtualization reviewer after five plan revisions. Every
# other gate in this lab inspects the renderer's own tables --
# `vkr_extension_table`, dispatch entries, pNext decoders. None of them looks at
# the capset, which is a *separate* advertisement channel reaching the guest
# before any command is dispatched.
#
# The mechanism, verified in the fork:
#
#   vkr_renderer.c            vn_info_extension_mask_init(ext_mask)
#                             memcpy(c->vk_extension_mask1, ext_mask, ...)
#
#   vn_protocol_renderer_info.h
#       vn_info_extension_mask_init() sets a bit for EVERY entry in
#       _vn_info_extensions, unconditionally. There is no filter for whether
#       the renderer actually supports the extension.
#
# W1 added the three video extensions to the protocol, so they are in that
# table, so their capset bits are set. A guest reading the capset is told Venus
# speaks video, while `vkr_extension_table` says otherwise and every other gate
# passes.
#
# This gate reports the bits that would be set. W2's implementation must mask
# the video numbers out until W3 deliberately flips capset advertisement.
#
# Usage: video-capset-gate.sh [<virglrenderer-dir>]

set -euo pipefail

STATE="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks"
VIRGL_DIR="${1:-${VIRGL_DIR:-$STATE/virglrenderer}}"

die() { echo "capset-gate: $*" >&2; exit 1; }

# Packaged into the store by flake.nix; the gate runs from /nix/store,
# so a $0-relative lookup resolves to the store root and not this tree.

INFO="$VIRGL_DIR/src/venus/venus-protocol/vn_protocol_renderer_info.h"
[ -f "$INFO" ] || die "no $INFO"

# Extension numbers the capset would advertise for video, read from the same
# generated table the mask is built from.
video_bits=$(
  sed -e 's://.*::' -e 's:/\*[^*]*\*\+\([^/*][^*]*\*\+\)*/::g' "$INFO" \
    | grep -oE '\{ *"VK_KHR_video_[a-z0-9_]+" *, *[0-9]+' \
    | sed -E 's/\{ *"([^"]+)" *, *([0-9]+)/\1 \2/' || true
)

if [ -z "$video_bits" ]; then
  echo "capset-gate: PASS -- no VK_KHR_video_* in the capset extension table"
  exit 0
fi

# The bits are in the table. That is only safe if the renderer clears them
# before filling the capset, so check both that every number is named AND that
# a clear operation exists that consumes them.
#
# Checking the annotations alone is not enough: removing the clear loop while
# leaving the annotated array in place passed this gate, which is the same
# "a check exists" reasoning that has produced every false pass in this lab.
# The bits are in the table. That is only safe if the renderer actually clears
# them before filling the capset.
#
# Two earlier shapes of this check both false-passed. Reading the numbers out of
# the annotated array literal and separately counting that *some* clear existed
# anywhere in src/venus credited a number for being *written down*, not for
# being cleared: truncating the loop to `i < 1` left ext 25 and 41 advertised to
# the guest while the gate printed all three as "cleared". That is the same
# credit-by-mention and split-obligation pair found in the enforcement and
# reply-hygiene gates -- on the gate guarding this wave's central claim.
#
# So bind the three facts that together mean "cleared", and refuse to infer any
# of them from the other two:
#   1. an array literal contains the number,
#   2. a loop iterates that array over its FULL ARRAY_SIZE, and
#   3. that loop's body masks the element off the capset array.
# INVERTED IN W3, and NARROWED to what a static check can honestly prove.
#
# The renderer no longer clears a hardcoded list of numbers. It walks the
# protocol table and clears every VK_KHR_video* extension NOT named in a
# supported set, so the default is deny and a newly serializable extension has
# to be named before it can reach a guest.
#
# capset-clear-check.py cannot verify that shape: it proves a number appears in
# an array literal that a full-extent loop masks off the capset. Against a
# derivation there is no literal to find, so it reports every number as
# uncleared -- which is why running it here reported the three IMPLEMENTED
# decode extensions as wrongly cleared. The helper was not wrong; it was being
# asked a question it does not answer.
#
# Rather than contort the helper, this gate now asserts the two things a static
# reader genuinely can:
#
#   1. The renderer's supported set is exactly the decode allowlist.
#   2. No ENCODE extension appears in it.
#
# Whether the guest actually SEES the three extensions is a runtime fact, and
# it is checked at runtime by the guest capability report rather than inferred
# from source here. Asserting a derived runtime outcome statically is precisely
# the credit-by-mention this lab has produced false passes with before.
supported_decl=$(
  find "$VIRGL_DIR/src/venus" -name '*.c' -print0 2>/dev/null \
    | xargs -0 sed -e 's://.*::' 2>/dev/null \
    | grep -oE '"VK_KHR_video_[a-z0-9_]+"' | tr -d '"' | sed 's/^VK_//' | sort -u
)

expected_supported="KHR_video_decode_h264
KHR_video_decode_queue
KHR_video_queue"

if [ "$supported_decl" != "$expected_supported" ]; then
  echo "capset-gate: FAIL -- the renderer supported-video set is not the decode allowlist" >&2
  diff <(printf '%s\n' "$expected_supported") <(printf '%s\n' "$supported_decl") \
    | sed 's/^/  /' >&2 || true
  echo >&2
  echo "  A name in this set is a claim the renderer can execute the WHOLE" >&2
  echo "  extension. Everything not named is cleared from the capset by the" >&2
  echo "  derivation, so adding a name here is what makes it reachable." >&2
  exit 1
fi

if printf '%s\n' "$supported_decl" | grep -qE 'encode'; then
  echo "capset-gate: FAIL -- an ENCODE extension is in the supported set:" >&2
  printf '%s\n' "$supported_decl" | grep -E 'encode' | sed 's/^/  /' >&2
  exit 1
fi

echo "capset-gate: PASS -- supported set is exactly the decode allowlist, encode absent"
printf '%s\n' "$supported_decl" | sed 's/^/  supported /'
printf '%s\n' "$video_bits" | sed 's/^/  in protocol table: /'
exit 0
